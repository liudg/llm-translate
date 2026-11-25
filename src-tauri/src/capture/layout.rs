//! Merge OCR lines into paragraphs and estimate their colours (DESIGN 5.6
//! steps 3 and 5).
//!
//! A line joins the paragraph above it when it sits directly below (gap under
//! ~0.7 line heights), has a similar height, starts near the same left edge,
//! and the line above is long and reaches the paragraph's right edge - i.e. it
//! looks wrapped. Short stacked lines such as menus, lists and table cells stay
//! separate so each keeps its own position.

use serde::Serialize;

use super::ocr::Line;
use crate::translate::detect::detect_paragraph;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paragraph {
    /// x, y, width, height in image pixels.
    pub rect: [f32; 4],
    /// Average OCR box height of its lines.
    pub line_height: f32,
    pub lines: usize,
    pub text: String,
    /// Contains letters in either language, so it goes to the model; others
    /// (numbers, symbols) stay as they are on the image.
    pub translatable: bool,
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    /// Width and height (from the rect's origin) the translation may grow
    /// into: plain background with no other text, at most twice the box.
    pub room: [f32; 2],
}

struct Group {
    lines: Vec<Line>,
}

impl Group {
    fn bounds(&self) -> (f32, f32, f32, f32) {
        let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for l in &self.lines {
            b = (b.0.min(l.x0), b.1.min(l.y0), b.2.max(l.x1), b.3.max(l.y1));
        }
        b
    }

    fn avg_height(&self) -> f32 {
        self.lines.iter().map(Line::height).sum::<f32>() / self.lines.len() as f32
    }

    /// `row`: the other OCR lines on `l`'s row.
    fn accepts(&self, l: &Line, row: &[&Line]) -> bool {
        let last = self.lines.last().unwrap();
        let h = self.avg_height();
        let (left, _, right, _) = self.bounds();
        // More text beside `l` within this paragraph's width: the row is laid
        // out in items (options, menus, table cells), not wrapped prose.
        if row.iter().any(|o| o.x0 < right && o.x1 > left) {
            return false;
        }
        let gap = l.y0 - last.y1;
        let ratio = l.height() / h;
        let last_width = last.x1 - last.x0;
        gap > -0.3 * h
            && gap < 0.7 * h
            && (0.75..=1.33).contains(&ratio)
            && (l.x0 - last.x0).abs() < 1.5 * h
            && last_width >= 8.0 * h
            && last.x1 >= right - 1.5 * h
            // Must overlap horizontally (not a neighbouring column).
            && l.x0 < last.x1
            && l.x1 > last.x0
    }

    fn text(&self) -> String {
        let mut out = String::new();
        for l in &self.lines {
            join_line(&mut out, &l.text);
        }
        out
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// Append a wrapped line: no space between CJK characters, a de-hyphenated
/// join for English words broken across lines, a space otherwise.
fn join_line(out: &mut String, next: &str) {
    let (Some(prev), Some(first)) = (out.chars().last(), next.chars().next()) else {
        out.push_str(next);
        return;
    };
    if prev == '-' && first.is_lowercase() && out.chars().rev().nth(1).is_some_and(|c| c.is_alphabetic()) {
        out.pop();
    } else if !(is_cjk(prev) || is_cjk(first)) {
        out.push(' ');
    }
    out.push_str(next);
}

/// Group lines into paragraphs in reading order (top to bottom, then left to right).
pub fn paragraphs(mut lines: Vec<Line>, bgra: &[u8], width: u32, height: u32) -> Vec<Paragraph> {
    lines.sort_by(|a, b| a.y0.total_cmp(&b.y0).then(a.x0.total_cmp(&b.x0)));
    // Lines beside each line: horizontally apart, vertically overlapping by
    // at least half the shorter one.
    let rows: Vec<Vec<usize>> = (0..lines.len())
        .map(|i| {
            let a = &lines[i];
            (0..lines.len())
                .filter(|&j| {
                    let b = &lines[j];
                    j != i
                        && (a.x1 <= b.x0 || b.x1 <= a.x0)
                        && a.y1.min(b.y1) - a.y0.max(b.y0) >= 0.5 * a.height().min(b.height())
                })
                .collect()
        })
        .collect();
    let mut groups: Vec<Group> = Vec::new();
    'lines: for (i, l) in lines.iter().enumerate() {
        let row: Vec<&Line> = rows[i].iter().map(|&j| &lines[j]).collect();
        // Only open groups whose last line ends near this line's top qualify.
        for g in groups.iter_mut().rev() {
            if g.accepts(l, &row) {
                g.lines.push(l.clone());
                continue 'lines;
            }
        }
        groups.push(Group { lines: vec![l.clone()] });
    }
    let img = Image { px: bgra, w: width as i32, h: height as i32 };
    let mut out: Vec<Paragraph> = groups
        .into_iter()
        .map(|g| {
            let (x0, y0, x1, y1) = g.bounds();
            let n = g.lines.len();
            let line_height = g.avg_height();
            let text = g.text();
            let (bg, fg) = img.colors(x0, y0, x1, y1);
            Paragraph {
                rect: [x0, y0, x1 - x0, y1 - y0],
                line_height,
                lines: n,
                translatable: detect_paragraph(&text).is_some(),
                text,
                bg,
                fg,
                room: [x1 - x0, y1 - y0],
            }
        })
        .collect();
    let rects: Vec<[f32; 4]> = out.iter().map(|p| p.rect).collect();
    for (i, p) in out.iter_mut().enumerate() {
        p.room = img.room(i, &rects, p.bg);
    }
    reading_order(out)
}

/// Top to bottom; paragraphs whose tops are within half a line of a row's
/// first paragraph form one visual row, read left to right. Grouping first
/// keeps the sort a total order (a pairwise "same row" comparator is not
/// transitive, and `sort_by` may panic on that; release builds abort).
fn reading_order(mut paras: Vec<Paragraph>) -> Vec<Paragraph> {
    paras.sort_by(|a, b| a.rect[1].total_cmp(&b.rect[1]));
    let mut out = Vec::with_capacity(paras.len());
    let mut row: Vec<Paragraph> = Vec::new();
    let mut row_top = 0.0f32;
    let mut row_line = f32::MAX;
    for p in paras {
        if !row.is_empty() && p.rect[1] - row_top >= 0.5 * row_line.min(p.line_height) {
            row.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
            out.append(&mut row);
        }
        if row.is_empty() {
            row_top = p.rect[1];
            row_line = p.line_height;
        } else {
            row_line = row_line.min(p.line_height);
        }
        row.push(p);
    }
    row.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
    out.append(&mut row);
    out
}

struct Image<'a> {
    px: &'a [u8],
    w: i32,
    h: i32,
}

impl Image<'_> {
    fn rgb(&self, x: i32, y: i32) -> [u8; 3] {
        let i = ((y * self.w + x) * 4) as usize;
        [self.px[i + 2], self.px[i + 1], self.px[i]]
    }

    /// Background: per-channel median of the ring 2 px outside the box.
    /// Text: median of the box pixels that differ most from the background.
    fn colors(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> ([u8; 3], [u8; 3]) {
        let clamp_x = |v: i32| v.clamp(0, self.w - 1);
        let clamp_y = |v: i32| v.clamp(0, self.h - 1);
        let (l, t) = (clamp_x(x0.floor() as i32 - 2), clamp_y(y0.floor() as i32 - 2));
        let (r, b) = (clamp_x(x1.ceil() as i32 + 1), clamp_y(y1.ceil() as i32 + 1));
        let mut ring = Vec::new();
        for x in l..=r {
            ring.push(self.rgb(x, t));
            ring.push(self.rgb(x, b));
        }
        for y in t..=b {
            ring.push(self.rgb(l, y));
            ring.push(self.rgb(r, y));
        }
        let bg = median(&mut ring);

        let mut inside: Vec<([u8; 3], u32)> = Vec::new();
        let step = (((r - l) * (b - t)) as f32 / 40_000.0).sqrt().max(1.0) as i32;
        for y in (t + 1..b).step_by(step as usize) {
            for x in (l + 1..r).step_by(step as usize) {
                let c = self.rgb(x, y);
                inside.push((c, distance(c, bg)));
            }
        }
        if inside.is_empty() {
            return (bg, contrast_fallback(bg));
        }
        inside.sort_by_key(|&(_, d)| std::cmp::Reverse(d));
        let top = (inside.len() / 10).max(1);
        if inside[0].1 < 40 * 40 {
            // Nothing stands out (blurry or very low contrast text).
            return (bg, contrast_fallback(bg));
        }
        let mut fg: Vec<[u8; 3]> = inside[..top].iter().map(|&(c, _)| c).collect();
        (bg, median(&mut fg))
    }
}

impl Image<'_> {
    /// How far paragraph `i` can grow right, then down, over background only.
    fn room(&self, i: usize, rects: &[[f32; 4]], bg: [u8; 3]) -> [f32; 2] {
        const GAP: i32 = 4;
        let [x, y, w, h] = rects[i];
        let (x0, y0) = (x.floor() as i32, y.floor() as i32);
        let (x1, y1) = ((x + w).ceil() as i32, (y + h).ceil() as i32);
        let is_bg = |px: i32, py: i32| distance(self.rgb(px, py), bg) <= 28 * 28;
        let hits_other = |l: i32, t: i32, r: i32, b: i32| {
            rects.iter().enumerate().any(|(j, o)| {
                j != i
                    && (l as f32) < o[0] + o[2] + GAP as f32
                    && (r as f32) > o[0] - GAP as f32
                    && (t as f32) < o[1] + o[3] + GAP as f32
                    && (b as f32) > o[1] - GAP as f32
            })
        };
        let (t, b) = (y0.clamp(0, self.h - 1), y1.clamp(0, self.h));
        let mut right = x1;
        let max_right = (x1 + (x1 - x0)).min(self.w - GAP);
        while right + 1 < max_right {
            let col = right + 1;
            if hits_other(col, t, col + 1, b) || !(t..b).step_by(2).all(|py| is_bg(col, py)) {
                right = (right - GAP).max(x1);
                break;
            }
            right = col;
        }
        let mut bottom = y1;
        let max_bottom = (y1 + (y1 - y0)).min(self.h - GAP);
        let (l, r) = (x0.clamp(0, self.w - 1), right.clamp(0, self.w));
        while bottom + 1 < max_bottom {
            let row = bottom + 1;
            if hits_other(l, row, r, row + 1) || !(l..r).step_by(2).all(|px| is_bg(px, row)) {
                bottom = (bottom - GAP).max(y1);
                break;
            }
            bottom = row;
        }
        [(right - x0).max(1) as f32, (bottom - y0).max(1) as f32]
    }
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    (0..3).map(|i| (a[i] as i32 - b[i] as i32).pow(2) as u32).sum()
}

fn median(v: &mut [[u8; 3]]) -> [u8; 3] {
    let mut out = [0u8; 3];
    for (ch, o) in out.iter_mut().enumerate() {
        let mid = v.len() / 2;
        v.select_nth_unstable_by_key(mid, |c| c[ch]);
        *o = v[mid][ch];
    }
    out
}

fn contrast_fallback(bg: [u8; 3]) -> [u8; 3] {
    let luma = 0.299 * bg[0] as f32 + 0.587 * bg[1] as f32 + 0.114 * bg[2] as f32;
    if luma > 128.0 { [0x1a, 0x1a, 0x1a] } else { [0xf2, 0xf2, 0xf2] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(x0: f32, y0: f32, x1: f32, y1: f32, text: &str) -> Line {
        Line { x0, y0, x1, y1, text: text.into(), confidence: 1.0 }
    }

    fn white(w: u32, h: u32) -> Vec<u8> {
        vec![255; (w * h * 4) as usize]
    }

    fn para(x: f32, y: f32, text: &str) -> Paragraph {
        Paragraph {
            rect: [x, y, 50.0, 20.0],
            line_height: 20.0,
            lines: 1,
            text: text.into(),
            translatable: true,
            bg: [255; 3],
            fg: [0; 3],
            room: [50.0, 20.0],
        }
    }

    #[test]
    fn staggered_tops_sort_without_cycles() {
        // Pairwise "same row" would say B<A, C<B but A<C: a cycle.
        let out = reading_order(vec![para(200.0, 10.0, "A"), para(100.0, 18.0, "B"), para(0.0, 26.0, "C")]);
        let order: Vec<&str> = out.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(order, ["B", "A", "C"]);
        // Many jittered boxes must not panic and must keep every paragraph.
        let many: Vec<Paragraph> =
            (0..200).map(|i| para(((i * 37) % 500) as f32, ((i * 7) % 60) as f32, "x")).collect();
        assert_eq!(reading_order(many).len(), 200);
    }

    #[test]
    fn wrapped_lines_merge_and_lists_stay_apart() {
        let lines = vec![
            line(10.0, 10.0, 600.0, 30.0, "Large language models run locally on consumer"),
            line(10.0, 34.0, 300.0, 54.0, "hardware with no network."),
            line(10.0, 100.0, 60.0, 120.0, "File"),
            line(10.0, 124.0, 60.0, 144.0, "Edit"),
        ];
        let p = paragraphs(lines, &white(700, 200), 700, 200);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].text, "Large language models run locally on consumer hardware with no network.");
        assert_eq!(p[0].lines, 2);
        assert_eq!(p[1].text, "File");
        assert_eq!(p[2].text, "Edit");
    }

    #[test]
    fn item_row_under_a_long_line_stays_apart() {
        // Real OCR boxes: a question, then options spread across one row.
        let lines = vec![
            line(64.0, 15.0, 743.0, 45.0, "How is Claude doing this session? (optional)"),
            line(67.0, 54.0, 164.0, 82.0, "1: Bad"),
            line(220.0, 54.0, 334.0, 82.0, "2: Fine"),
            line(374.0, 55.0, 488.0, 81.0, "3: Good"),
            line(527.0, 54.0, 684.0, 82.0, "0: Dismiss"),
        ];
        let p = paragraphs(lines, &white(802, 110), 802, 110);
        let texts: Vec<&str> = p.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, ["How is Claude doing this session? (optional)", "1: Bad", "2: Fine", "3: Good", "0: Dismiss"]);
        assert_eq!(p[0].lines, 1);
    }

    #[test]
    fn two_column_prose_still_merges() {
        let lines = vec![
            line(10.0, 10.0, 400.0, 30.0, "left column text that is long enough"),
            line(500.0, 10.0, 900.0, 30.0, "right column text that is long enough"),
            line(10.0, 34.0, 200.0, 54.0, "left ends here."),
            line(500.0, 34.0, 700.0, 54.0, "right ends here."),
        ];
        let p = paragraphs(lines, &white(1000, 100), 1000, 100);
        let texts: Vec<&str> = p.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            texts,
            ["left column text that is long enough left ends here.", "right column text that is long enough right ends here."]
        );
    }

    #[test]
    fn join_rules() {
        let mut s = String::from("trans-");
        join_line(&mut s, "lation");
        assert_eq!(s, "translation");
        let mut s = String::from("本地翻译");
        join_line(&mut s, "模型");
        assert_eq!(s, "本地翻译模型");
        let mut s = String::from("well-");
        join_line(&mut s, "Known");
        assert_eq!(s, "well- Known");
    }

    #[test]
    fn columns_do_not_merge() {
        let lines = vec![
            line(10.0, 10.0, 400.0, 30.0, "left column text that is long enough"),
            line(500.0, 34.0, 900.0, 54.0, "right column text that is long enough"),
        ];
        assert_eq!(paragraphs(lines, &white(1000, 100), 1000, 100).len(), 2);
    }

    #[test]
    fn room_stops_at_content_and_other_text() {
        let (w, h) = (400u32, 200u32);
        let mut px = vec![255u8; (w * h * 4) as usize];
        // a dark bar at x 300..310 to the right of the first line
        for y in 0..h {
            for x in 300..310 {
                let i = ((y * w + x) * 4) as usize;
                px[i..i + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        let lines = vec![line(10.0, 10.0, 110.0, 30.0, "Hello"), line(10.0, 60.0, 50.0, 80.0, "Next")];
        let p = paragraphs(lines, &px, w, h);
        // right: up to the bar minus the gap; down: until the next paragraph
        assert!(p[0].room[0] > 150.0 && p[0].room[0] < 290.0, "{:?}", p[0].room);
        assert!(p[0].room[1] >= 20.0 && p[0].room[1] < 50.0, "{:?}", p[0].room);
    }

    #[test]
    fn colors_dark_text_on_light() {
        let (w, h) = (100u32, 40u32);
        let mut px = vec![240u8; (w * h * 4) as usize];
        for y in 15..25 {
            for x in 20..80 {
                let i = ((y * w + x) * 4) as usize;
                px[i..i + 3].copy_from_slice(&[30, 20, 10]); // BGR
            }
        }
        let img = Image { px: &px, w: w as i32, h: h as i32 };
        let (bg, fg) = img.colors(15.0, 10.0, 85.0, 30.0);
        assert_eq!(bg, [240, 240, 240]);
        assert_eq!(fg, [10, 20, 30]);
    }
}
