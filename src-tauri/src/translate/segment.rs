//! Segmentation (DESIGN 5.3): each non-empty line is a paragraph; line breaks and
//! indentation are kept verbatim; a paragraph over ~1500 tokens is split by sentence.

pub const MAX_SEGMENT_TOKENS: usize = 1500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// Text to translate, already split into chunks small enough for one request.
    Text(Vec<String>),
    /// Copied to the output unchanged (line breaks, indentation, blank lines).
    Keep(String),
}

/// Rough token estimate: one per CJK char, ~3.5 chars per token otherwise.
pub fn estimate_tokens(s: &str) -> usize {
    let (mut cjk, mut other) = (0usize, 0usize);
    for c in s.chars() {
        if (c as u32) >= 0x2E80 {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    cjk + other * 2 / 7 + 1
}

fn sentence_end(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '；' | '.' | '!' | '?' | ';' | '…')
}

/// Split a long paragraph at sentence ends, packing sentences up to `max` tokens.
fn split_long(p: &str, max: usize) -> Vec<String> {
    if estimate_tokens(p) <= max {
        return vec![p.to_string()];
    }
    let mut sentences = Vec::new();
    let mut cur = String::new();
    let mut chars = p.chars().peekable();
    while let Some(c) = chars.next() {
        cur.push(c);
        let ascii_end = matches!(c, '.' | '!' | '?' | ';');
        if sentence_end(c) && (!ascii_end || chars.peek().is_none_or(|n| n.is_whitespace())) {
            while let Some(&n) = chars.peek() {
                if n.is_whitespace() || matches!(n, '”' | '"' | '’' | '\'' | '）' | ')') {
                    cur.push(n);
                    chars.next();
                } else {
                    break;
                }
            }
            sentences.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        sentences.push(cur);
    }
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    for s in sentences {
        if !buf.is_empty() && estimate_tokens(&buf) + estimate_tokens(&s) > max {
            out.push(std::mem::take(&mut buf));
        }
        // A single sentence longer than max is hard-cut by characters.
        if estimate_tokens(&s) > max {
            let chars: Vec<char> = s.chars().collect();
            for c in chars.chunks(max) {
                out.push(c.iter().collect());
            }
            continue;
        }
        buf.push_str(&s);
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

pub fn segment(text: &str) -> Vec<Piece> {
    let text = text.replace("\r\n", "\n");
    let mut pieces = Vec::new();
    let mut keep = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            keep.push('\n');
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            keep.push_str(line);
            continue;
        }
        let lead = &line[..line.len() - line.trim_start().len()];
        keep.push_str(lead);
        if !keep.is_empty() {
            pieces.push(Piece::Keep(std::mem::take(&mut keep)));
        }
        pieces.push(Piece::Text(split_long(trimmed, MAX_SEGMENT_TOKENS)));
    }
    if !keep.is_empty() {
        pieces.push(Piece::Keep(keep));
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Piece {
        Piece::Text(vec![s.into()])
    }
    fn k(s: &str) -> Piece {
        Piece::Keep(s.into())
    }

    #[test]
    fn keeps_structure() {
        assert_eq!(segment("Hello"), vec![t("Hello")]);
        assert_eq!(
            segment("第一段\n\n  第二段 \r\n第三段\n"),
            vec![t("第一段"), k("\n\n  "), t("第二段"), k("\n"), t("第三段"), k("\n")]
        );
        assert!(segment("  \n ").iter().all(|p| matches!(p, Piece::Keep(_))));
    }

    #[test]
    fn splits_long_paragraph_by_sentence() {
        let sentence = "这是一个用于测试分段功能的比较长的句子，它包含很多汉字。";
        let para = sentence.repeat(80); // ~2240 tokens
        let pieces = segment(&para);
        let Piece::Text(chunks) = &pieces[0] else { panic!() };
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| estimate_tokens(c) <= MAX_SEGMENT_TOKENS));
        assert!(chunks.iter().all(|c| c.ends_with('。')));
        assert_eq!(chunks.concat(), para);

        let en = "This is a sentence about version 1.5 of the app. ".repeat(300);
        let Piece::Text(chunks) = &segment(&en)[0] else { panic!() };
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| c.ends_with("app.")));
    }
}
