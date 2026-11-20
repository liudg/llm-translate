// Draws translations over the screenshot (DESIGN 5.6 step 5). Everything is in
// physical image pixels: the canvas backing store is the screenshot itself.
import type { ShotParagraph } from "../../lib/api";

const FONT = '"Segoe UI", "Microsoft YaHei UI", sans-serif';
/** Floor for the shrink-to-fit font size, in DIPs. */
const MIN_FONT_DIP = 9;
/** Line advance relative to the font size. */
const LINE_GAP = 1.28;
/** Erase this far beyond the OCR box. */
const PAD = 2;

const CJK = "⺀-鿿豈-﫿＀-￯";
const TOKEN = new RegExp(`[${CJK}]|[^\\s${CJK}]+|\\s+`, "g");
/** May not start a line (kinsoku): attach to the previous one instead. */
const NO_LINE_START = new Set("，。、；：？！）》」』】”’,.;:?!)]}%…·");

interface Layout {
  font: number;
  gap: number;
  lines: string[];
  /** Widest line and block height, in pixels. */
  width: number;
  height: number;
}

function wrap(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string[] {
  const lines: string[] = [];
  let line = "";
  let space = false;
  const push = () => {
    if (line) lines.push(line);
    line = "";
    space = false;
  };
  for (const tok of text.match(TOKEN) ?? []) {
    if (/^\s+$/.test(tok)) {
      if (tok.includes("\n")) push();
      else space = line.length > 0;
      continue;
    }
    const candidate = line + (space ? " " : "") + tok;
    if (!line || ctx.measureText(candidate).width <= maxWidth || NO_LINE_START.has(tok[0])) {
      if (!line && ctx.measureText(tok).width > maxWidth) {
        // A single word wider than the box: break it by characters.
        for (const ch of tok) {
          if (line && ctx.measureText(line + ch).width > maxWidth) push();
          line += ch;
        }
      } else {
        line = candidate;
      }
    } else {
      push();
      line = tok;
    }
    space = false;
  }
  push();
  return lines;
}

/** Font size relative to the OCR box height (measured: 0.73-1.16, mostly ~0.95). */
const FONT_PER_BOX = 0.9;
/** Height a single line needs, relative to the font size. */
const LINE_BOX = 1.15;

/** Height of `n` lines: full advances between them, one line box for the last. */
const blockHeight = (n: number, font: number, gap: number) => (n - 1) * gap + font * LINE_BOX;

/** Largest font, from `start` down to `min`, whose wrapped text fits w x h. */
function fitBox(ctx: CanvasRenderingContext2D, text: string, w: number, h: number, start: number, min: number): Layout {
  let font = start;
  for (;;) {
    ctx.font = `${font}px ${FONT}`;
    const lines = wrap(ctx, text, w);
    const gap = Math.round(font * LINE_GAP);
    const height = blockHeight(lines.length, font, gap);
    if (height <= h * 1.1 || font <= min) {
      const width = Math.max(...lines.map((l) => ctx.measureText(l).width));
      return { font, gap, lines, width, height };
    }
    font = Math.max(min, Math.floor(font * 0.94));
  }
}

/**
 * Fit the original box; when that needs a much smaller font than the original
 * (typically Chinese to English), grow into the free background around it.
 */
function fit(ctx: CanvasRenderingContext2D, p: ShotParagraph, text: string, dpr: number): Layout & { grown: boolean } {
  const [, , w, h] = p.rect;
  const min = Math.max(1, Math.round(MIN_FONT_DIP * dpr));
  const start = Math.max(min, Math.round(p.lineHeight * FONT_PER_BOX));
  const inBox = fitBox(ctx, text, w, h, start, min);
  const [rw, rh] = p.room;
  if (inBox.font >= start * 0.85 || (rw <= w && rh <= h)) return { ...inBox, grown: false };
  const grown = fitBox(ctx, text, Math.max(w, rw), Math.max(h, rh), start, min);
  return grown.font > inBox.font ? { ...grown, grown: true } : { ...inBox, grown: false };
}

const rgb = (c: [number, number, number]) => `rgb(${c[0]} ${c[1]} ${c[2]})`;

export function drawParagraph(
  ctx: CanvasRenderingContext2D,
  p: ShotParagraph,
  translation: string,
  dpr: number,
  imageHeight: number,
) {
  const text = translation.trim();
  if (!text) return;
  const [x, y, w, h] = p.rect;
  const { font, gap, lines, width, height, grown } = fit(ctx, p, text, dpr);
  // Erase the original box plus whatever the text covers beyond it; text
  // that still does not fit at the minimum size runs on downward.
  const boxWidth = Math.max(w, Math.ceil(width));
  const boxHeight = Math.min(Math.max(h, height), imageHeight - y);
  ctx.fillStyle = rgb(p.bg);
  ctx.fillRect(x - PAD, y - PAD, boxWidth + 2 * PAD, boxHeight + 2 * PAD);
  ctx.fillStyle = rgb(p.fg);
  ctx.font = `${font}px ${FONT}`;
  ctx.textBaseline = "middle";
  // Centred in the original box; grown text starts at its top instead.
  const top = grown ? y : y + Math.max(0, (h - height) / 2);
  lines.forEach((line, i) => ctx.fillText(line, x, top + i * gap + (font * LINE_BOX) / 2));
}
