import iconUrl from "../../../../Logo/final-v2/png/icon-256.png";
import jakartaUrl from "@fontsource-variable/plus-jakarta-sans/files/plus-jakarta-sans-latin-wght-normal.woff2?url";
import jakartaExtUrl from "@fontsource-variable/plus-jakarta-sans/files/plus-jakarta-sans-latin-ext-wght-normal.woff2?url";
import { flowBands, type FlowSpec } from "./flowGeometry";
import {
  CARD_SIZES,
  type CardDeck,
  type CardFormat,
  type CardKind,
  type CardSlide,
} from "./shareContent";
import {
  FLOW_PATH,
  LOCKUP_ICON,
  LOCKUP_WIDTH,
  SPEAKO_PATH,
  WORDMARK_ORIGIN,
} from "./wordmark";

/**
 * The share cards: the Insights numbers as images to post anywhere.
 *
 * They are drawn on a canvas rather than captured from the page, so the
 * preview and the saved file are the same pixels, and nothing depends on
 * rasterising HTML, which WebKit refuses to export (an SVG `foreignObject`
 * taints the canvas there). What each card says is decided in
 * `shareContent.ts`; this file only lays it out.
 *
 * Every card is one colour field with the icon's flowing bands crossing it
 * in tones of that colour, one number set large straight on the field in the
 * logo's own typeface, a line or two about it, and the logo. Nothing sits in
 * a box: a card that reads as a screenshot of a dashboard is a card nobody
 * posts.
 */

const FAMILY = "SpeakoFlow Card";
const FONT = `"${FAMILY}", "Inter Variable", system-ui, -apple-system, "Segoe UI", sans-serif`;

interface Palette {
  ground: string;
  /** The colour the bands lean toward, a step at a time. */
  band: string;
  /** How far each band leans toward `band`, outermost first. */
  steps: number[];
  ink: string;
  /** Labels, notes and the tagline. */
  soft: string;
  /** Hairlines and empty cells. */
  faint: string;
  /** The "Flow" of the wordmark. */
  flow: string;
}

const PALETTES: Record<CardKind, Palette> = {
  highlights: {
    ground: "#0c7a6e",
    band: "#7af7df",
    steps: [0.14, 0.24, 0.34],
    ink: "#ffffff",
    soft: "rgba(255, 255, 255, 0.8)",
    faint: "rgba(255, 255, 255, 0.24)",
    flow: "#a8fff0",
  },
  words: {
    ground: "#d8ccff",
    band: "#6a46f2",
    steps: [0.12, 0.2, 0.29],
    ink: "#1c1050",
    soft: "rgba(28, 16, 80, 0.72)",
    faint: "rgba(28, 16, 80, 0.16)",
    flow: "#0b7469",
  },
  speed: {
    ground: "#2a4ee3",
    band: "#a9bbff",
    steps: [0.14, 0.23, 0.32],
    ink: "#ffffff",
    soft: "rgba(255, 255, 255, 0.8)",
    faint: "rgba(255, 255, 255, 0.26)",
    flow: "#b3fff0",
  },
  saved: {
    ground: "#ffc387",
    band: "#ff6a12",
    steps: [0.13, 0.22, 0.31],
    ink: "#3a1500",
    soft: "rgba(58, 21, 0, 0.72)",
    faint: "rgba(58, 21, 0, 0.16)",
    flow: "#3a1500",
  },
  streak: {
    ground: "#7a3fe0",
    band: "#d8b8ff",
    steps: [0.14, 0.23, 0.32],
    ink: "#ffffff",
    soft: "rgba(255, 255, 255, 0.8)",
    faint: "rgba(255, 255, 255, 0.24)",
    flow: "#c9fff4",
  },
  assistant: {
    ground: "#bcdcff",
    band: "#1f74f0",
    steps: [0.12, 0.2, 0.29],
    ink: "#06264d",
    soft: "rgba(6, 38, 77, 0.72)",
    faint: "rgba(6, 38, 77, 0.16)",
    flow: "#06264d",
  },
  meetings: {
    ground: "#b3256c",
    band: "#ffa6d3",
    steps: [0.14, 0.23, 0.32],
    ink: "#ffffff",
    soft: "rgba(255, 255, 255, 0.8)",
    faint: "rgba(255, 255, 255, 0.24)",
    flow: "#ffd3e8",
  },
  activity: {
    ground: "#bdf1da",
    band: "#079a77",
    steps: [0.12, 0.2, 0.29],
    ink: "#043a2d",
    soft: "rgba(4, 58, 45, 0.72)",
    faint: "rgba(4, 58, 45, 0.14)",
    flow: "#087a66",
  },
};

/** The ground of each card, for the dialog while a card is being drawn. */
export const CARD_GROUNDS: Record<CardKind, string> = Object.fromEntries(
  Object.entries(PALETTES).map(([kind, palette]) => [kind, palette.ground]),
) as Record<CardKind, string>;

/** The ribbon's three bands, the same on every card. */
const RIBBON = [-0.2, -0.06, 0.05, 0.19];

/**
 * The S each card's ribbon follows, so a deck does not repeat itself. It
 * passes behind the words, in tones close enough to the ground that the
 * words never have to fight it, the way the icon's waves sit behind its
 * voice bars.
 */
const SPINES: Record<CardKind, FlowSpec["spine"]> = {
  highlights: [
    [1.15, 0.12],
    [0.35, -0.02],
    [0.68, 1.02],
    [-0.15, 0.88],
  ],
  words: [
    [-0.15, 0.15],
    [0.55, 0],
    [0.45, 1],
    [1.15, 0.85],
  ],
  speed: [
    [-0.2, 0.55],
    [0.4, -0.15],
    [0.75, 1.15],
    [1.2, 0.4],
  ],
  saved: [
    [0.25, -0.2],
    [1.15, 0.3],
    [-0.05, 0.7],
    [0.85, 1.2],
  ],
  streak: [
    [-0.15, 0.85],
    [0.35, 0.3],
    [0.75, 0.95],
    [1.15, 0.05],
  ],
  assistant: [
    [1.2, 0.62],
    [0.6, -0.12],
    [0.3, 1.1],
    [-0.2, 0.35],
  ],
  meetings: [
    [0.62, -0.2],
    [1.3, 0.45],
    [0.1, 0.6],
    [0.66, 1.2],
  ],
  // Across the top corner only, so the grid of days stays on a plain ground.
  activity: [
    [0.3, -0.25],
    [0.55, 0.32],
    [0.95, 0.12],
    [1.25, 0.5],
  ],
};

interface Assets {
  icon: HTMLImageElement;
}

let assetsPromise: Promise<Assets> | null = null;

const loadImage = (src: string) =>
  new Promise<HTMLImageElement>((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error(`Could not load ${src}`));
    image.src = src;
  });

/** The typeface and the icon, loaded once. A face that fails to load falls
 *  back to Inter rather than failing the card. */
export const loadCardAssets = (): Promise<Assets> => {
  if (!assetsPromise) {
    const faces = [
      new FontFace(FAMILY, `url(${jakartaUrl})`, {
        weight: "200 800",
        unicodeRange:
          "U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD",
      }),
      new FontFace(FAMILY, `url(${jakartaExtUrl})`, {
        weight: "200 800",
        unicodeRange:
          "U+0100-02BA, U+02BD-02C5, U+02C7-02CC, U+02CE-02D7, U+02DD-02FF, U+1D00-1DBF, U+1E00-1E9F, U+1EF2-1EFF, U+2020, U+20A0-20AB, U+20AD-20C0, U+2113, U+2C60-2C7F, U+A720-A7FF",
      }),
    ];
    const fonts = Promise.all(
      faces.map((face) =>
        face
          .load()
          .then((loaded) => document.fonts.add(loaded))
          .catch(() => undefined),
      ),
    );
    assetsPromise = Promise.all([fonts, loadImage(iconUrl)])
      .then(([, icon]) => ({ icon }))
      .catch((error) => {
        assetsPromise = null;
        throw error;
      });
  }
  return assetsPromise;
};

/* ───────────────────────────── helpers ───────────────────────────── */

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * A right-to-left language reads the card from the right, so the card is
 * drawn as the mirror image of its left-to-right layout: text aligned right,
 * the logo at the bottom right, the activity grid running from today on the
 * left. Layout is written left to right; these two turn it around.
 */
const frame = { rtl: false, width: 0 };

/** The left edge of a box `w` wide whose left-to-right left edge is `x`. */
const mx = (x: number, w = 0): number => (frame.rtl ? frame.width - x - w : x);

/** Text alignment, turned around for a right-to-left card. */
const flip = (align: CanvasTextAlign): CanvasTextAlign => {
  if (!frame.rtl) return align;
  if (align === "left") return "right";
  if (align === "right") return "left";
  return align;
};

const setTracking = (
  ctx: CanvasRenderingContext2D,
  em: number,
  size: number,
) => {
  if ("letterSpacing" in ctx) {
    (
      ctx as CanvasRenderingContext2D & { letterSpacing: string }
    ).letterSpacing = `${(em * size).toFixed(2)}px`;
  }
};

const font = (weight: number, size: number) =>
  `${weight} ${Math.round(size)}px ${FONT}`;

const roundRect = (
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) => {
  const radius = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + radius, y);
  ctx.arcTo(x + w, y, x + w, y + h, radius);
  ctx.arcTo(x + w, y + h, x, y + h, radius);
  ctx.arcTo(x, y + h, x, y, radius);
  ctx.arcTo(x, y, x + w, y, radius);
  ctx.closePath();
};

/** `color` moved `amount` of the way toward `toward`. */
const mix = (color: string, toward: string, amount: number): string => {
  const parse = (hex: string) => {
    const value = parseInt(hex.slice(1), 16);
    return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
  };
  const a = parse(color);
  const b = parse(toward);
  const channel = (i: number) => Math.round(a[i] + (b[i] - a[i]) * amount);
  return `rgb(${channel(0)}, ${channel(1)}, ${channel(2)})`;
};

/** Break `text` into lines no wider than `max`. Splits at spaces, or between
 *  characters for scripts written without them. */
const wrap = (
  ctx: CanvasRenderingContext2D,
  text: string,
  max: number,
): string[] => {
  const spaced = /\s/.test(text.trim());
  const tokens = spaced ? text.trim().split(/\s+/) : Array.from(text.trim());
  const joiner = spaced ? " " : "";
  const lines: string[] = [];
  let line = "";
  for (const token of tokens) {
    const next = line ? `${line}${joiner}${token}` : token;
    if (ctx.measureText(next).width <= max || !line) {
      line = next;
    } else {
      lines.push(line);
      line = token;
    }
  }
  if (line) lines.push(line);
  return lines;
};

/** `wrap`, but text that needs two lines is split where the two come out
 *  closest in length, so a sentence never ends on one word alone. */
const wrapBalanced = (
  ctx: CanvasRenderingContext2D,
  text: string,
  max: number,
): string[] => {
  const lines = wrap(ctx, text, max);
  if (lines.length !== 2) return lines;
  const spaced = /\s/.test(text.trim());
  const tokens = spaced ? text.trim().split(/\s+/) : Array.from(text.trim());
  const joiner = spaced ? " " : "";
  let best = lines;
  let bestWidth = Math.max(...lines.map((line) => ctx.measureText(line).width));
  for (let split = 1; split < tokens.length; split += 1) {
    const pair = [
      tokens.slice(0, split).join(joiner),
      tokens.slice(split).join(joiner),
    ];
    const widest = Math.max(...pair.map((line) => ctx.measureText(line).width));
    if (widest <= max && widest < bestWidth) {
      best = pair;
      bestWidth = widest;
    }
  }
  return best;
};

/* ───────────────────────────── figures ───────────────────────────── */

/**
 * A figure is set as its digits at full size and any words in it ("hr",
 * "min", "K", "万") at a third of that, on the same baseline, so "8 hr 11
 * min" reads as two numbers rather than one long word.
 */
interface Run {
  text: string;
  unit: boolean;
}

const runsOf = (text: string): Run[] => {
  const runs: Run[] = [];
  for (const part of text.match(/[\d.,٫٬  ]+|\s+|[^\d.,\s]+/g) ?? [text]) {
    const unit = !/^[\d.,٫٬  ]+$/.test(part) || /^\s+$/.test(part);
    runs.push({ text: part, unit });
  }
  return runs;
};

const UNIT_SCALE = 0.34;
const FIGURE_WEIGHT = 700;
const FIGURE_TRACKING = -0.045;

const runFont = (run: Run, size: number) =>
  run.unit ? font(620, size * UNIT_SCALE) : font(FIGURE_WEIGHT, size);

const runTracking = (run: Run) => (run.unit ? -0.01 : FIGURE_TRACKING);

const figureWidth = (
  ctx: CanvasRenderingContext2D,
  text: string,
  size: number,
): number =>
  runsOf(text).reduce((sum, run) => {
    ctx.font = runFont(run, size);
    setTracking(ctx, runTracking(run), size * (run.unit ? UNIT_SCALE : 1));
    return sum + ctx.measureText(run.text).width;
  }, 0);

/** The largest size up to `size` at which the figure fits in `width`. */
const fitFigure = (
  ctx: CanvasRenderingContext2D,
  text: string,
  width: number,
  size: number,
): number => {
  let fitted = size;
  while (figureWidth(ctx, text, fitted) > width && fitted > 14) {
    fitted -= Math.max(1, Math.round(fitted * 0.03));
  }
  return fitted;
};

/** Draw a figure starting at `left` (left to right) on `baseline`. Returns
 *  its width. */
const drawFigure = (
  ctx: CanvasRenderingContext2D,
  text: string,
  left: number,
  baseline: number,
  size: number,
  color: string,
): number => {
  const runs = runsOf(text);
  const widths = runs.map((run) => {
    ctx.font = runFont(run, size);
    setTracking(ctx, runTracking(run), size * (run.unit ? UNIT_SCALE : 1));
    return ctx.measureText(run.text).width;
  });
  const total = widths.reduce((a, b) => a + b, 0);
  ctx.fillStyle = color;
  ctx.textAlign = "left";
  // Bidirectional text keeps its own order inside a run; a right-to-left
  // card lays the runs out from the right.
  let x = frame.rtl ? mx(left, total) + total : left;
  runs.forEach((run, index) => {
    ctx.font = runFont(run, size);
    setTracking(ctx, runTracking(run), size * (run.unit ? UNIT_SCALE : 1));
    if (frame.rtl) {
      x -= widths[index];
      ctx.fillText(run.text, x, baseline);
    } else {
      ctx.fillText(run.text, x, baseline);
      x += widths[index];
    }
  });
  setTracking(ctx, 0, 1);
  return total;
};

/** Cap height of the figure face, as a share of its size. */
const CAP = 0.72;

/* ───────────────────────────── the frame ───────────────────────────── */

/** The flowing bands, laid out in `box`: the top of the card down to just
 *  above its words. */
const drawFlow = (
  ctx: CanvasRenderingContext2D,
  spec: FlowSpec,
  palette: Palette,
  box: Box,
) => {
  const bands = flowBands(spec, box.w, box.h);
  bands.forEach((band, index) => {
    ctx.beginPath();
    band.forEach(([x, y], point) => {
      const px = mx(box.x + x);
      if (point === 0) ctx.moveTo(px, box.y + y);
      else ctx.lineTo(px, box.y + y);
    });
    ctx.closePath();
    ctx.fillStyle = mix(
      palette.ground,
      palette.band,
      palette.steps[Math.min(index, palette.steps.length - 1)],
    );
    ctx.fill();
  });
};

/** The logo at `height` tall with its bottom on `bottom`, its words in the
 *  card's own colours. Returns its width. */
const logo = (
  ctx: CanvasRenderingContext2D,
  assets: Assets,
  palette: Palette,
  x: number,
  bottom: number,
  height: number,
): number => {
  const scale = height / LOCKUP_ICON;
  const width = LOCKUP_WIDTH * scale;
  // A logo is never mirrored: in a right-to-left card it moves to the
  // other corner as it is.
  const left = mx(x, width);
  const top = bottom - height;
  ctx.drawImage(assets.icon, left, top, height, height);
  ctx.save();
  ctx.translate(
    left + WORDMARK_ORIGIN.x * scale,
    top + WORDMARK_ORIGIN.y * scale,
  );
  ctx.scale(scale, scale);
  ctx.fillStyle = palette.ink;
  ctx.fill(new Path2D(SPEAKO_PATH));
  ctx.fillStyle = palette.flow;
  ctx.fill(new Path2D(FLOW_PATH));
  ctx.restore();
  return width;
};

interface Layout {
  pad: number;
  /** Where the words go: they stand on its bottom edge. */
  region: Box;
  /** The gap kept between the bands and the words. */
  clearance: number;
  footer: { bottom: number; logo: number; tagline: number };
  /** The largest a figure is set. */
  figure: number;
  /** Scale for the type around the figure. */
  k: number;
}

const layoutFor = (format: CardFormat, w: number, h: number): Layout => {
  if (format === "story") {
    // Instagram and Facebook draw their own bars over about the top and
    // bottom 250 pixels of a story, so the words and the logo stay inside.
    const pad = 92;
    const bottom = h - 290;
    const logo = 62;
    return {
      pad,
      region: { x: pad, y: 290, w: w - pad * 2, h: bottom - logo - 150 - 290 },
      clearance: 120,
      footer: { bottom, logo, tagline: 28 },
      figure: 330,
      k: 1.2,
    };
  }
  if (format === "wide") {
    const pad = 96;
    const logo = 52;
    return {
      pad,
      region: { x: pad, y: pad, w: 900, h: h - pad - logo - 76 - pad },
      clearance: 70,
      footer: { bottom: h - pad + 6, logo, tagline: 26 },
      figure: 230,
      k: 0.9,
    };
  }
  const pad = 92;
  const logo = 56;
  return {
    pad,
    region: { x: pad, y: pad, w: w - pad * 2, h: h - pad - logo - 92 - pad },
    clearance: 90,
    footer: { bottom: h - pad + 6, logo, tagline: 27 },
    figure: 260,
    k: 1,
  };
};

/* ───────────────────────────── the content ───────────────────────────── */

interface Ctx {
  ctx: CanvasRenderingContext2D;
  palette: Palette;
  layout: Layout;
  format: CardFormat;
  deck: CardDeck;
}

/** Lines of text, wrapped to `max`, at most two of them. */
interface TextBlock {
  rows: string[];
  size: number;
  weight: number;
  color: string;
  tracking: number;
  /** Space above, as a share of the size. */
  gap: number;
}

const textBlock = (
  c: Ctx,
  text: string | undefined,
  size: number,
  weight: number,
  color: string,
  max: number,
  gap: number,
  tracking = -0.012,
): TextBlock | null => {
  if (!text) return null;
  const { ctx } = c;
  ctx.font = font(weight, size);
  setTracking(ctx, tracking, size);
  return {
    rows: wrapBalanced(ctx, text, max).slice(0, 2),
    size,
    weight,
    color,
    tracking,
    gap,
  };
};

const LINE = 1.22;

const blockHeight = (block: TextBlock | null) =>
  block ? block.gap * block.size + block.rows.length * block.size * LINE : 0;

/** Draw a text block with its top at `top`; returns its bottom. */
const drawText = (
  c: Ctx,
  block: TextBlock | null,
  left: number,
  top: number,
): number => {
  if (!block) return top;
  const { ctx } = c;
  ctx.font = font(block.weight, block.size);
  setTracking(ctx, block.tracking, block.size);
  ctx.fillStyle = block.color;
  ctx.textAlign = flip("left");
  let y = top + block.gap * block.size;
  for (const row of block.rows) {
    ctx.fillText(row, mx(left), y + block.size * 0.92);
    y += block.size * LINE;
  }
  setTracking(ctx, 0, 1);
  return y;
};

/** The card's title, set over its figure. */
const kicker = (c: Ctx, slide: CardSlide, width: number) =>
  textBlock(c, slide.title, 40 * c.layout.k, 620, c.palette.ink, width, 0);

/**
 * What a card says, in its lower part: how tall it stands, and how to draw
 * it with its top at a given height. The card stands it on the bottom of its
 * region and gives the bands the room above.
 */
interface Piece {
  height: number;
  draw: (top: number) => void;
}

interface Extra {
  height: number;
  gap: number;
  draw: (top: number) => void;
}

/** The title, the figure, what it counts and a sentence: words, time saved,
 *  the streak, the assistant and meetings. `extra` goes under the words. */
const figurePiece = (
  c: Ctx,
  slide: CardSlide,
  region: Box,
  extra?: Extra,
): Piece => {
  const { ctx, palette, layout } = c;
  const k = layout.k;
  const title = kicker(c, slide, region.w);
  const unitSize = slide.unit ? layout.figure * 0.3 : 0;
  ctx.font = font(640, unitSize);
  setTracking(ctx, -0.02, unitSize);
  const unitW = slide.unit
    ? ctx.measureText(slide.unit).width + unitSize * 0.3
    : 0;
  const size = fitFigure(
    ctx,
    slide.value ?? "",
    region.w - unitW,
    layout.figure,
  );
  const label = textBlock(
    c,
    slide.label,
    48 * k,
    640,
    palette.ink,
    region.w,
    0.45,
  );
  const note = textBlock(
    c,
    slide.note,
    34 * k,
    520,
    palette.soft,
    region.w,
    0.5,
  );
  const titleGap = 0.38 * size;
  const height =
    blockHeight(title) +
    titleGap +
    size * CAP +
    size * 0.05 +
    blockHeight(label) +
    blockHeight(note) +
    (extra ? extra.gap + extra.height : 0);
  return {
    height,
    draw: (top) => {
      let y = drawText(c, title, region.x, top);
      const baseline = y + titleGap + size * CAP;
      const width = drawFigure(
        ctx,
        slide.value ?? "",
        region.x,
        baseline,
        size,
        palette.ink,
      );
      if (slide.unit) {
        ctx.font = font(640, unitSize);
        setTracking(ctx, -0.02, unitSize);
        ctx.fillStyle = palette.ink;
        ctx.textAlign = flip("left");
        ctx.fillText(
          slide.unit,
          mx(region.x + width + unitSize * 0.3),
          baseline,
        );
        setTracking(ctx, 0, 1);
      }
      y = drawText(c, label, region.x, baseline + size * 0.05);
      y = drawText(c, note, region.x, y);
      if (extra) extra.draw(y + extra.gap);
    },
  };
};

/** Highlights: up to four figures, two by two, each under a hairline. */
const highlightsPiece = (c: Ctx, slide: CardSlide, region: Box): Piece => {
  const { ctx, palette, layout, format } = c;
  const items = slide.items ?? [];
  const k = layout.k;
  const columns = 2;
  const rows = Math.ceil(items.length / columns);
  const colGap = 64 * k;
  const cellW = (region.w - colGap * (columns - 1)) / columns;
  const max = (format === "story" ? 150 : format === "wide" ? 112 : 124) * k;
  // Every figure is set at the size the widest allows, so they read as one
  // set.
  const size = Math.min(
    ...items.map((item) => fitFigure(ctx, item.value, cellW, max)),
  );
  const labels = items.map((item) =>
    textBlock(c, item.label, 32 * k, 560, palette.soft, cellW, 0.4),
  );
  const ruleGap = 30 * k;
  const cellH = ruleGap + size * CAP + Math.max(...labels.map(blockHeight));
  const rowGap = (format === "story" ? 60 : 48) * k;
  const title = kicker(c, slide, region.w);
  const titleGap = 44 * k;
  const height =
    blockHeight(title) + titleGap + rows * cellH + (rows - 1) * rowGap;
  return {
    height,
    draw: (top) => {
      const gridTop = drawText(c, title, region.x, top) + titleGap;
      items.forEach((item, index) => {
        const col = index % columns;
        const row = Math.floor(index / columns);
        const x = region.x + col * (cellW + colGap);
        const y = gridTop + row * (cellH + rowGap);
        ctx.fillStyle = palette.faint;
        ctx.fillRect(mx(x, cellW), y, cellW, Math.max(2, 2 * k));
        const baseline = y + ruleGap + size * CAP;
        drawFigure(ctx, item.value, x, baseline, size, palette.ink);
        drawText(c, labels[index], x, baseline + size * 0.02);
      });
    },
  };
};

/** The speed: the figure, and a bar for it over a fainter one for a typical
 *  typist, who is a reference rather than the user. */
const speedPiece = (c: Ctx, slide: CardSlide, region: Box): Piece => {
  const { ctx, palette, layout } = c;
  const compare = slide.compare;
  const k = layout.k;
  const barH = 18 * k;
  const rowLabel = 28 * k;
  const rowH = rowLabel * 1.55 + barH;
  const rowGap = 26 * k;
  const barsW = Math.min(region.w, 780 * k);
  const bars: Extra | undefined = compare && {
    height: rowH * 2 + rowGap,
    gap: 56 * k,
    draw: (top) => {
      const max = Math.max(compare.you, compare.typing);
      let y = top;
      [
        { label: compare.youLabel, value: compare.you, color: palette.ink },
        {
          label: compare.typingLabel,
          value: compare.typing,
          color: palette.faint,
        },
      ].forEach((row, index) => {
        ctx.font = font(560, rowLabel);
        setTracking(ctx, -0.01, rowLabel);
        ctx.fillStyle = index === 0 ? palette.ink : palette.soft;
        ctx.textAlign = flip("left");
        ctx.fillText(row.label, mx(region.x), y + rowLabel);
        ctx.textAlign = flip("right");
        ctx.font = font(680, rowLabel);
        ctx.fillText(String(row.value), mx(region.x + barsW), y + rowLabel);
        setTracking(ctx, 0, 1);
        y += rowLabel * 1.55;
        const length = Math.max(barH, (barsW * row.value) / max);
        roundRect(ctx, mx(region.x, length), y, length, barH, barH / 2);
        ctx.fillStyle = row.color;
        ctx.fill();
        y += barH + rowGap;
      });
      ctx.textAlign = "left";
    },
  };
  return figurePiece(c, { ...slide, note: undefined }, region, bars);
};

/** The days active over six months of them, one square each: under the
 *  figure, or beside it across the whole of a wide card. */
const activityPiece = (c: Ctx, slide: CardSlide, region: Box): Piece => {
  const { ctx, palette, layout, format } = c;
  const grid = slide.activity;
  const k = layout.k;
  if (!grid) return figurePiece(c, slide, region);
  const weeks = grid.weeks.length;
  const wide = format === "wide";
  const monthSize = 24 * k;
  const monthBand = monthSize * 1.7;
  const textW = wide ? 380 : region.w;
  const between = 72;
  const gridRoom = wide
    ? frame.width - layout.pad - (region.x + textW + between)
    : region.w;
  const pitch = Math.min(gridRoom / weeks, 40 * k);
  const cell = pitch * 0.8;
  const gridW = pitch * weeks - (pitch - cell);
  const gridH = pitch * 7 - (pitch - cell);
  // Opaque, so a cell reads the same over a band as over the ground.
  const levels = [
    mix(palette.ground, palette.ink, 0.09),
    mix(palette.ground, palette.band, 0.32),
    mix(palette.ground, palette.band, 0.55),
    mix(palette.ground, palette.band, 0.8),
    palette.band,
  ];
  const drawGrid = (left: number, gridTop: number) => {
    ctx.font = font(560, monthSize);
    ctx.fillStyle = palette.soft;
    ctx.textAlign = flip("left");
    grid.months.forEach(({ col, label }) => {
      if (col > weeks - 2) return;
      ctx.fillText(label, mx(left + col * pitch), gridTop - monthSize * 0.7);
    });
    grid.weeks.forEach((column, week) => {
      column.forEach((level, day) => {
        if (level < 0) return;
        roundRect(
          ctx,
          mx(left + week * pitch, cell),
          gridTop + day * pitch,
          cell,
          cell,
          cell * 0.3,
        );
        ctx.fillStyle = levels[level];
        ctx.fill();
      });
    });
    ctx.textAlign = "left";
  };
  if (!wide) {
    return figurePiece(c, slide, region, {
      height: monthBand + gridH,
      gap: 52 * k,
      draw: (top) => drawGrid(region.x, top + monthBand),
    });
  }
  const text = figurePiece(c, slide, { ...region, w: textW });
  const height = Math.max(text.height, monthBand + gridH);
  return {
    height,
    draw: (top) => {
      text.draw(top + height - text.height);
      drawGrid(frame.width - layout.pad - gridW, top + height - gridH);
    },
  };
};

/* ───────────────────────────── the card ───────────────────────────── */

/** Draw one card at its full size onto `canvas`, mirrored for a
 *  right-to-left language. Assets must be loaded. */
export const drawCard = (
  canvas: HTMLCanvasElement,
  slide: CardSlide,
  deck: CardDeck,
  format: CardFormat,
  assets: Assets,
  rtl = false,
) => {
  const { width: w, height: h } = CARD_SIZES[format];
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  frame.rtl = rtl;
  frame.width = w;
  ctx.direction = rtl ? "rtl" : "ltr";
  ctx.textBaseline = "alphabetic";
  const palette = PALETTES[slide.kind];
  const layout = layoutFor(format, w, h);
  const c: Ctx = { ctx, palette, layout, format, deck };
  const { region } = layout;

  const piece =
    slide.kind === "highlights"
      ? highlightsPiece(c, slide, region)
      : slide.kind === "speed"
        ? speedPiece(c, slide, region)
        : slide.kind === "activity"
          ? activityPiece(c, slide, region)
          : figurePiece(c, slide, region);
  const top = region.y + Math.max(0, region.h - piece.height);

  ctx.fillStyle = palette.ground;
  ctx.fillRect(0, 0, w, h);
  drawFlow(
    ctx,
    { spine: SPINES[slide.kind], edges: RIBBON, fan: [1, 1] },
    palette,
    { x: 0, y: 0, w, h },
  );
  piece.draw(top);

  const { footer } = layout;
  const logoWidth = logo(
    ctx,
    assets,
    palette,
    layout.pad,
    footer.bottom,
    footer.logo,
  );
  const taglineRoom = w - layout.pad * 2 - logoWidth - footer.tagline * 2;
  for (let size = footer.tagline; size >= footer.tagline * 0.72; size -= 1) {
    ctx.font = font(540, size);
    if (ctx.measureText(deck.tagline).width <= taglineRoom) {
      ctx.fillStyle = palette.soft;
      ctx.textAlign = flip("right");
      ctx.fillText(
        deck.tagline,
        mx(w - layout.pad),
        footer.bottom - footer.logo / 2 + size * 0.36,
      );
      break;
    }
  }
  ctx.textAlign = "left";
};

/** The card as a PNG. */
export const cardBlob = (canvas: HTMLCanvasElement): Promise<Blob> =>
  new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => (blob ? resolve(blob) : reject(new Error("No image"))),
      "image/png",
    );
  });

/** A blob as base64, without the data-URL prefix, for the save command. */
export const blobToBase64 = (blob: Blob): Promise<string> =>
  new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = String(reader.result ?? "");
      resolve(result.slice(result.indexOf(",") + 1));
    };
    reader.onerror = () => reject(reader.error ?? new Error("Read failed"));
    reader.readAsDataURL(blob);
  });
