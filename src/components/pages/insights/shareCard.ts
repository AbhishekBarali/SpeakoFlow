import iconUrl from "../../../../Logo/final-v2/png/icon-256.png";
import displayFontUrl from "@/assets/fonts/hubot-sans-latin-standard.woff2?url";
import digitsFontUrl from "@fontsource-variable/inter/files/inter-latin-wght-normal.woff2?url";
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
 * Every card is built the same way, so a deck of them reads as one set: a
 * flat field in its own colour, the SpeakoFlow icon's seven voice bars drawn
 * huge in a tone of it and hanging from the top edge (their heights are the
 * user's last seven days), the card's title, one white panel sized to what it
 * holds, and the logo with the tagline along the foot. One number per card
 * and plenty of room around it: a card that reads as an advert, or as a page
 * of statistics, is a card nobody posts.
 */

const DISPLAY_FAMILY = "SpeakoFlow Display";
/** Inter's digits in front of the display face: its zero is drawn with a
 *  stroke through it, which reads as a theta at this size. The website
 *  makes the same swap. */
const DIGITS_FAMILY = "SpeakoFlow Digits";
const DISPLAY = `"${DIGITS_FAMILY}", "${DISPLAY_FAMILY}", "Inter Variable", system-ui, sans-serif`;
const TEXT = `"Inter Variable", system-ui, -apple-system, "Segoe UI", sans-serif`;
const HEADING = `"${DIGITS_FAMILY}", "${DISPLAY_FAMILY}", ${TEXT}`;

/** The panel's own ink, the same on every card. */
const PANEL_INK = "#14181b";
const PANEL_MUTED = "#5d6569";

/** Tracking of a figure; tighter and the digits of a duration run into its
 *  units at feed size. */
const FIGURE_TRACKING = -0.03;

interface Palette {
  ground: string;
  /** The voice bars, a tone of the ground. */
  bars: string;
  /** The title and the "Speako" of the wordmark. */
  ink: string;
  /** The "Flow" of the wordmark. */
  flow: string;
  tagline: string;
  /** The speed card's own bar. */
  accent: string;
}

const PALETTES: Record<CardKind, Palette> = {
  highlights: {
    ground: "#d9d0fb",
    bars: "#ccc0f8",
    ink: "#211a57",
    flow: "#0b6f64",
    tagline: "rgba(33, 26, 87, 0.8)",
    accent: "#5b4fd6",
  },
  words: {
    ground: "#0c6a63",
    bars: "#137a71",
    ink: "#ffffff",
    flow: "#8ff2df",
    tagline: "rgba(255, 255, 255, 0.84)",
    accent: "#0f766e",
  },
  speed: {
    ground: "#15191c",
    bars: "#21272b",
    ink: "#ffffff",
    flow: "#5eead4",
    tagline: "rgba(255, 255, 255, 0.76)",
    accent: "#14b8a6",
  },
  saved: {
    ground: "#f6b07a",
    bars: "#efa169",
    ink: "#3a1c0b",
    flow: "#3a1c0b",
    tagline: "rgba(58, 28, 11, 0.82)",
    accent: "#c2410c",
  },
  streak: {
    ground: "#5b48d9",
    bars: "#6654de",
    ink: "#ffffff",
    flow: "#b9f5ea",
    tagline: "rgba(255, 255, 255, 0.84)",
    accent: "#5b48d9",
  },
  assistant: {
    ground: "#f4d771",
    bars: "#ebca5a",
    ink: "#362b06",
    flow: "#362b06",
    tagline: "rgba(54, 43, 6, 0.82)",
    accent: "#a16207",
  },
  meetings: {
    ground: "#f4adc9",
    bars: "#ee9fbf",
    ink: "#47132d",
    flow: "#47132d",
    tagline: "rgba(71, 19, 45, 0.82)",
    accent: "#be185d",
  },
  activity: {
    ground: "#1c4a34",
    bars: "#24583f",
    ink: "#ffffff",
    flow: "#a7f3d0",
    tagline: "rgba(255, 255, 255, 0.82)",
    accent: "#15803d",
  },
};

/** The ground of each card, for the dialog while a card is being drawn. */
export const CARD_GROUNDS: Record<CardKind, string> = Object.fromEntries(
  Object.entries(PALETTES).map(([kind, palette]) => [kind, palette.ground]),
) as Record<CardKind, string>;

/** The activity grid's five levels, empty to busiest. */
const HEAT = ["#e6ecec", "#bfe8e1", "#7fd5c6", "#2fb3a1", "#0d7268"];

/** The SpeakoFlow icon's voice bars, for a week with nothing in it. */
const ICON_BARS = [0.38, 0.7, 1, 0.58, 0.84, 0.46, 0.26];

/** Where each card's bars hang across the top, so a deck does not repeat
 *  itself: the centre of the seven, as a share of the card's width. */
const BAR_CENTRE: Record<CardKind, number> = {
  highlights: 0.74,
  words: 0.8,
  speed: 0.7,
  saved: 0.78,
  streak: 0.72,
  assistant: 0.8,
  meetings: 0.7,
  activity: 0.76,
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

/** Fonts and the icon, loaded once. A face that fails to load falls back to
 *  Inter rather than failing the card. */
export const loadCardAssets = (): Promise<Assets> => {
  if (!assetsPromise) {
    const faces = [
      new FontFace(DISPLAY_FAMILY, `url(${displayFontUrl})`, {
        weight: "600 720",
        stretch: "100% 102%",
      }),
      new FontFace(DIGITS_FAMILY, `url(${digitsFontUrl})`, {
        weight: "100 900",
        unicodeRange: "U+0030-0039",
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

const font = (weight: number, size: number, family: string) =>
  `${weight} ${Math.round(size)}px ${family}`;

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

/** The largest size up to `size` at which `text` fits in `width`. Leaves
 *  the font and tracking set. */
const fitSize = (
  ctx: CanvasRenderingContext2D,
  text: string,
  width: number,
  size: number,
  weight: number,
  family: string,
  tracking: number,
): number => {
  let fitted = size;
  for (;;) {
    ctx.font = font(weight, fitted, family);
    setTracking(ctx, tracking, fitted);
    if (ctx.measureText(text).width <= width || fitted <= 14) return fitted;
    fitted -= Math.max(1, Math.round(fitted * 0.03));
  }
};

/** A caption in capitals, the way a figure's label is set. */
const caps = (text: string, locale: string): string => {
  try {
    return text.toLocaleUpperCase(locale);
  } catch {
    return text.toUpperCase();
  }
};

/* ───────────────────────────── figures ───────────────────────────── */

/**
 * A figure is set as its digits at full size and any words in it ("hr",
 * "min", "K", "万") at less than half, on the same baseline, so "8 hr 11 min"
 * reads as two numbers rather than one long word.
 */
interface Run {
  text: string;
  unit: boolean;
}

const runsOf = (text: string): Run[] => {
  const runs: Run[] = [];
  for (const part of text.match(/[\d.,٫٬  ]+|\s+|[^\d.,\s]+/g) ?? [text]) {
    const unit = !/^[\d.,٫٬  ]+$/.test(part) || /^\s+$/.test(part);
    runs.push({ text: part, unit });
  }
  return runs;
};

const UNIT_SCALE = 0.44;

const runFont = (run: Run, size: number) =>
  run.unit ? font(650, size * UNIT_SCALE, HEADING) : font(680, size, DISPLAY);

const figureWidth = (
  ctx: CanvasRenderingContext2D,
  text: string,
  size: number,
): number =>
  runsOf(text).reduce((sum, run) => {
    ctx.font = runFont(run, size);
    setTracking(ctx, run.unit ? 0 : FIGURE_TRACKING, size);
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

/** Draw a figure starting at `left` (left to right) on `baseline`. */
const drawFigure = (
  ctx: CanvasRenderingContext2D,
  text: string,
  left: number,
  baseline: number,
  size: number,
  color: string,
) => {
  const runs = runsOf(text);
  const widths = runs.map((run) => {
    ctx.font = runFont(run, size);
    setTracking(ctx, run.unit ? 0 : FIGURE_TRACKING, size);
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
    setTracking(ctx, run.unit ? 0 : FIGURE_TRACKING, size);
    if (frame.rtl) {
      x -= widths[index];
      ctx.fillText(run.text, x, baseline);
    } else {
      ctx.fillText(run.text, x, baseline);
      x += widths[index];
    }
  });
  setTracking(ctx, 0, 1);
};

/* ───────────────────────────── the frame ───────────────────────────── */

/**
 * The seven voice bars of the SpeakoFlow icon, in a tone of the ground. Their
 * heights are the user's last seven days, so the mark is theirs. They never
 * touch a panel or the band the logo sits in: beside the panel where a card
 * leaves room at its side (a wide card), otherwise hanging from the top edge
 * and ending a margin above the panel.
 */
const voiceBars = (
  ctx: CanvasRenderingContext2D,
  w: number,
  kind: CardKind,
  color: string,
  week: number[],
  floor: number,
  margin: number,
  around: Box,
) => {
  const busiest = Math.max(0, ...week);
  const heights =
    week.length === 7 && busiest > 0
      ? week.map((value) => 0.3 + 0.7 * Math.sqrt(value))
      : ICON_BARS;
  const count = heights.length;
  const besideRoom = w - (around.x + around.w) - margin;
  ctx.save();
  ctx.fillStyle = color;
  if (besideRoom > w * 0.25) {
    // Standing in the room beside the panel, whole, from the top edge to
    // above the foot.
    const left = around.x + around.w + margin;
    const length = floor * 0.9;
    const width = Math.min(
      length * 0.115,
      (besideRoom * 0.9) / (count + (count - 1) * 0.6),
    );
    const gap = width * 0.6;
    const span = count * width + (count - 1) * gap;
    const start = left + (besideRoom - span) / 2;
    const middle = floor * 0.46;
    ctx.beginPath();
    ctx.rect(mx(left, w - left), 0, w - left, floor);
    ctx.clip();
    heights.forEach((height, index) => {
      const barH = length * height;
      roundRect(
        ctx,
        mx(start + index * (width + gap), width),
        middle - barH / 2,
        width,
        barH,
        width / 2,
      );
      ctx.fill();
    });
  } else {
    // Hanging from the top edge, the longest ending a margin above the
    // panel, so the mark is cropped only by the card itself.
    const limit = Math.min(around.y, floor) - margin;
    if (limit > margin) {
      const overhang = 0.42;
      const length = limit / (1 - overhang);
      const width = Math.min(
        length * 0.2,
        (w * 0.44) / (count + (count - 1) * 0.6),
      );
      const gap = width * 0.6;
      const span = count * width + (count - 1) * gap;
      const left = Math.min(
        BAR_CENTRE[kind] * w - span / 2,
        w - span - margin * 0.5,
      );
      heights.forEach((height, index) => {
        roundRect(
          ctx,
          mx(left + index * (width + gap), width),
          -length * overhang,
          width,
          length * height,
          width / 2,
        );
        ctx.fill();
      });
    }
  }
  ctx.restore();
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

/** A white panel for a number to sit on. */
const panel = (ctx: CanvasRenderingContext2D, box: Box, radius: number) => {
  ctx.save();
  ctx.shadowColor = "rgba(10, 14, 20, 0.14)";
  ctx.shadowBlur = radius * 1.2;
  ctx.shadowOffsetY = radius * 0.35;
  roundRect(ctx, mx(box.x, box.w), box.y, box.w, box.h, radius);
  ctx.fillStyle = "#ffffff";
  ctx.fill();
  ctx.restore();
};

interface Layout {
  pad: number;
  title: { size: number; baseline: number };
  /** Where the panel may go; it takes the height its content needs and is
   *  centred in this. */
  region: Box;
  radius: number;
  footer: { bottom: number; logo: number; tagline: number };
  /** The band the voice bars stay above. */
  floor: number;
  /** Scale for everything set inside the panel. */
  k: number;
  /** A story sets its title right above the panel and centres the two. */
  grouped: boolean;
}

const layoutFor = (format: CardFormat, w: number, h: number): Layout => {
  if (format === "story") {
    // Instagram and Facebook draw their own bars over about the top and
    // bottom 250 pixels of a story, so the title and the logo stay inside.
    const pad = 84;
    const top = 270;
    const bottom = h - 270;
    const logo = 60;
    return {
      pad,
      title: { size: 76, baseline: top + 64 },
      region: { x: pad, y: top, w: w - pad * 2, h: bottom - logo - 90 - top },
      radius: 56,
      footer: { bottom, logo, tagline: 30 },
      floor: bottom - logo - 60,
      k: 1.5,
      grouped: true,
    };
  }
  if (format === "wide") {
    const pad = 80;
    const logo = 48;
    const footerTop = h - pad - logo;
    const top = pad + 50 + pad;
    return {
      pad,
      title: { size: 58, baseline: pad + 50 },
      // The panel keeps to the left three fifths; the voice bars have the
      // rest.
      region: { x: pad, y: top, w: 880, h: footerTop - pad - top },
      radius: 44,
      footer: { bottom: h - pad + 4, logo, tagline: 26 },
      floor: footerTop - 40,
      k: 1,
      grouped: false,
    };
  }
  const pad = 84;
  const logo = 52;
  const footerTop = h - pad - logo;
  return {
    pad,
    title: { size: 62, baseline: pad + 54 },
    region: {
      x: pad,
      y: pad + 100,
      w: w - pad * 2,
      h: footerTop - 48 - (pad + 100),
    },
    radius: 44,
    footer: { bottom: h - pad + 4, logo, tagline: 26 },
    floor: footerTop - 40,
    k: 1,
    grouped: false,
  };
};

/* ───────────────────────────── the panels ───────────────────────────── */

interface Ctx {
  ctx: CanvasRenderingContext2D;
  palette: Palette;
  locale: string;
  format: CardFormat;
  k: number;
  radius: number;
  /** The most height a panel can have. */
  room: number;
  /** The box for a panel `height` tall; `full` takes the card's whole
   *  width where the panel otherwise keeps to part of it. */
  place: (height: number, full?: boolean) => Box;
}

/** One line of a centred block. */
interface Line {
  text: string;
  size: number;
  weight: number;
  family: string;
  color: string;
  tracking: number;
  /** Space above this line, in units of its size. */
  gap: number;
  /** The figure itself: digits and units, shrunk to the width. */
  figure?: boolean;
  /** A word set after the figure on its baseline, as in "12 days". */
  unit?: { text: string; size: number };
}

interface SetLine {
  line: Line;
  size: number;
  rows: string[];
}

interface Block {
  set: SetLine[];
  height: number;
}

const unitWidth = (ctx: CanvasRenderingContext2D, unit: Line["unit"]) => {
  if (!unit) return 0;
  ctx.font = font(650, unit.size, TEXT);
  setTracking(ctx, 0.06, unit.size);
  return ctx.measureText(unit.text).width + unit.size * 0.45;
};

const advance = (line: Line, size: number) =>
  line.figure ? size * 0.74 : size * 1.24;

/** Fit and wrap `lines` to `width`, and say how tall they stand. */
const measure = (c: Ctx, width: number, lines: Line[]): Block => {
  const { ctx } = c;
  const set = lines.map((line): SetLine => {
    if (line.figure) {
      const size = fitFigure(
        ctx,
        line.text,
        width - unitWidth(ctx, line.unit),
        line.size,
      );
      return { line, size, rows: [line.text] };
    }
    ctx.font = font(line.weight, line.size, line.family);
    setTracking(ctx, line.tracking, line.size);
    return {
      line,
      size: line.size,
      rows: wrapBalanced(ctx, line.text, width).slice(0, 2),
    };
  });
  const height = set.reduce(
    (sum, { line, size, rows }) =>
      sum + line.gap * size + advance(line, size) * rows.length,
    0,
  );
  return { set, height };
};

/** Draw a measured block centred in `box`. */
const drawBlock = (c: Ctx, box: Box, block: Block) => {
  const { ctx } = c;
  let y = box.y + (box.h - block.height) / 2;
  for (const { line, size, rows } of block.set) {
    y += line.gap * size;
    for (const row of rows) {
      const baseline = y + (line.figure ? size * 0.74 : size * 0.95);
      if (line.figure) {
        const valueWidth = figureWidth(ctx, row, size);
        const extra = unitWidth(ctx, line.unit);
        const start = box.x + (box.w - valueWidth - extra) / 2;
        drawFigure(ctx, row, start, baseline, size, line.color);
        if (line.unit) {
          ctx.font = font(650, line.unit.size, TEXT);
          setTracking(ctx, 0.06, line.unit.size);
          ctx.fillStyle = line.color;
          ctx.textAlign = flip("left");
          ctx.fillText(
            line.unit.text,
            mx(start + valueWidth + line.unit.size * 0.45),
            baseline,
          );
        }
      } else {
        ctx.font = font(line.weight, size, line.family);
        setTracking(ctx, line.tracking, size);
        ctx.fillStyle = line.color;
        ctx.textAlign = "center";
        ctx.fillText(row, mx(box.x + box.w / 2), baseline);
      }
      y += advance(line, size);
    }
  }
  setTracking(ctx, 0, 1);
  ctx.textAlign = "left";
};

const valueLine = (
  c: Ctx,
  text: string,
  size: number,
  unit?: string,
): Line => ({
  text,
  size: size * c.k,
  weight: 680,
  family: DISPLAY,
  color: PANEL_INK,
  tracking: FIGURE_TRACKING,
  gap: 0,
  figure: true,
  unit: unit ? { text: caps(unit, c.locale), size: 54 * c.k } : undefined,
});

const labelLine = (c: Ctx, text: string, gap: number): Line => ({
  text: caps(text, c.locale),
  size: 27 * c.k,
  weight: 600,
  family: TEXT,
  color: PANEL_MUTED,
  tracking: 0.14,
  gap,
});

const noteLine = (c: Ctx, text: string, gap: number): Line => ({
  text,
  size: 38 * c.k,
  weight: 600,
  family: HEADING,
  color: PANEL_INK,
  tracking: -0.01,
  gap,
});

/** The figure, what it counts, and a sentence: words, time saved, the
 *  streak, the assistant and meetings. */
const figureCard = (c: Ctx, slide: CardSlide, region: Box) => {
  const { k } = c;
  const padX = 64 * k;
  const padY = 84 * k;
  const lines: Line[] = [];
  if (slide.value) lines.push(valueLine(c, slide.value, 220, slide.unit));
  if (slide.label) lines.push(labelLine(c, slide.label, 1.5));
  if (slide.note) {
    lines.push(noteLine(c, slide.note, slide.label ? 0.95 : 1.15));
  }
  const block = measure(c, region.w - padX * 2, lines);
  const box = c.place(block.height + padY * 2);
  panel(c.ctx, box, c.radius);
  drawBlock(
    c,
    { x: box.x + padX, y: box.y, w: box.w - padX * 2, h: box.h },
    block,
  );
};

/** The speed: the figure, and a bar for it over a hairline one for a
 *  typical typist, who is a reference rather than the user. Side by side on
 *  a wide card, one above the other on the others. */
const speedCard = (c: Ctx, slide: CardSlide, region: Box) => {
  const { ctx, k, palette } = c;
  const compare = slide.compare;
  const wide = c.format === "wide";
  const padX = 64 * k;
  const padY = (wide ? 60 : 76) * k;
  const innerW = region.w - padX * 2;
  const figureW = wide ? innerW * 0.5 : innerW;
  const figure = measure(c, figureW, [
    valueLine(c, slide.value ?? "", wide ? 190 : 210),
    labelLine(c, slide.label ?? "", 1.5),
  ]);
  const labelSize = 28 * k;
  const barH = (wide ? 36 : 44) * k;
  const rowGap = 26 * k;
  const noteSize = (wide ? 32 : 36) * k;
  const barsH =
    2 * (labelSize * 1.5 + barH) + rowGap + (slide.note ? noteSize * 1.9 : 0);
  const between = 64 * k;
  const contentH = wide
    ? Math.max(figure.height, barsH)
    : figure.height + between + barsH;
  const box = c.place(contentH + padY * 2);
  panel(ctx, box, c.radius);
  const inner = {
    x: box.x + padX,
    y: box.y + padY,
    w: innerW,
    h: box.h - padY * 2,
  };
  drawBlock(
    c,
    wide
      ? { x: inner.x, y: inner.y, w: figureW, h: inner.h }
      : { x: inner.x, y: inner.y, w: inner.w, h: figure.height },
    figure,
  );
  if (!compare) return;
  const bars: Box = wide
    ? {
        x: inner.x + innerW * 0.56,
        y: inner.y + (inner.h - barsH) / 2,
        w: innerW * 0.44,
        h: barsH,
      }
    : {
        x: inner.x,
        y: inner.y + figure.height + between,
        w: inner.w,
        h: barsH,
      };
  const max = Math.max(compare.you, compare.typing);
  let y = bars.y;
  [
    { label: compare.youLabel, value: compare.you, reference: false },
    { label: compare.typingLabel, value: compare.typing, reference: true },
  ].forEach((row, index) => {
    ctx.font = font(600, labelSize, TEXT);
    setTracking(ctx, 0, 1);
    ctx.fillStyle = row.reference ? PANEL_MUTED : PANEL_INK;
    ctx.textAlign = flip("left");
    ctx.fillText(row.label, mx(bars.x), y + labelSize);
    ctx.textAlign = flip("right");
    ctx.font = font(680, labelSize, DISPLAY);
    ctx.fillText(String(row.value), mx(bars.x + bars.w), y + labelSize);
    y += labelSize * 1.5;
    const length = Math.max(barH, (bars.w * row.value) / max);
    if (row.reference) {
      roundRect(
        ctx,
        mx(bars.x, length) + 1.5,
        y + 1.5,
        length - 3,
        barH - 3,
        barH / 2,
      );
      ctx.strokeStyle = "#b9c2c3";
      ctx.lineWidth = 3;
      ctx.setLineDash([10 * k, 8 * k]);
      ctx.stroke();
      ctx.setLineDash([]);
    } else {
      roundRect(ctx, mx(bars.x, length), y, length, barH, barH / 2);
      ctx.fillStyle = palette.accent;
      ctx.fill();
    }
    y += barH + (index === 0 ? rowGap : 0);
  });
  if (slide.note) {
    ctx.font = font(600, noteSize, HEADING);
    ctx.fillStyle = PANEL_INK;
    ctx.textAlign = flip("left");
    ctx.fillText(slide.note, mx(bars.x), y + noteSize * 1.6);
  }
  ctx.textAlign = "left";
};

/** Highlights: small white panels instead of one, two by two, or one under
 *  another on a story. */
const highlightsCard = (c: Ctx, slide: CardSlide, region: Box) => {
  const { ctx, k } = c;
  const items = slide.items ?? [];
  const columns = c.format === "story" ? 1 : 2;
  const rows = Math.ceil(items.length / columns);
  const gap = 28 * k;
  const cellW = (region.w - gap * (columns - 1)) / columns;
  const padX = 48 * k;
  const labelSize = 26 * k;
  const valueMax = (c.format === "story" ? 92 : 104) * k;
  // Every cell is set at the size the widest figure allows, so they read as
  // one set.
  const valueSize = Math.min(
    ...items.map((item) =>
      fitFigure(ctx, item.value, cellW - padX * 2, valueMax),
    ),
  );
  const labels = items.map((item) => {
    ctx.font = font(600, labelSize, TEXT);
    setTracking(ctx, 0.12, labelSize);
    return wrap(ctx, caps(item.label, c.locale), cellW - padX * 2).slice(0, 2);
  });
  const lines = Math.max(...labels.map((rowLines) => rowLines.length));
  const contentH =
    valueSize * 0.74 + labelSize * 1.7 + (lines - 1) * labelSize * 1.3;
  const cellH = Math.min(
    (c.room - gap * (rows - 1)) / rows,
    contentH + 64 * k * 2,
  );
  const block = c.place(rows * cellH + (rows - 1) * gap);
  items.forEach((item, index) => {
    const col = index % columns;
    const row = Math.floor(index / columns);
    const cell: Box = {
      x: block.x + col * (cellW + gap),
      y: block.y + row * (cellH + gap),
      w: cellW,
      h: cellH,
    };
    panel(ctx, cell, c.radius * 0.8);
    const y = cell.y + (cell.h - contentH) / 2;
    drawFigure(
      ctx,
      item.value,
      cell.x + padX,
      y + valueSize * 0.74,
      valueSize,
      PANEL_INK,
    );
    ctx.font = font(600, labelSize, TEXT);
    setTracking(ctx, 0.12, labelSize);
    ctx.fillStyle = PANEL_MUTED;
    ctx.textAlign = flip("left");
    labels[index].forEach((line, lineIndex) => {
      ctx.fillText(
        line,
        mx(cell.x + padX),
        y + valueSize * 0.74 + labelSize * 1.7 + lineIndex * labelSize * 1.3,
      );
    });
    setTracking(ctx, 0, 1);
    ctx.textAlign = "left";
  });
};

/** The days active and six months of them, one square each: the figure
 *  over the grid, or beside it across the whole of a wide card. */
const activityCard = (c: Ctx, slide: CardSlide, region: Box) => {
  const { ctx, k } = c;
  const grid = slide.activity;
  if (!grid) return;
  const wide = c.format === "wide";
  const weeks = grid.weeks.length;
  const padX = 56 * k;
  const padY = 60 * k;
  const monthSize = 22 * k;
  const monthBand = monthSize * 1.9;
  const fullW = wide ? frame.width - region.x * 2 : region.w;
  const figureW = wide ? 300 * k : fullW - padX * 2;
  const figure = measure(c, figureW, [
    valueLine(c, slide.value ?? "", wide || c.format === "story" ? 150 : 132),
    labelLine(c, slide.label ?? "", 1.4),
  ]);
  const between = (wide ? 64 : 48) * k;
  const gridRoomW = wide
    ? fullW - padX * 2 - figureW - between
    : fullW - padX * 2;
  const fixedH = wide ? monthBand : figure.height + between + monthBand;
  const gridRoomH = c.room - padY * 2 - fixedH;
  const pitch = Math.min(gridRoomW / weeks, gridRoomH / 7, 54);
  const cell = pitch * 0.84;
  const gridW = pitch * weeks - (pitch - cell);
  const gridH = pitch * 7 - (pitch - cell);
  const contentH = wide
    ? Math.max(figure.height, monthBand + gridH)
    : figure.height + between + monthBand + gridH;
  const box = c.place(contentH + padY * 2, wide);
  panel(ctx, box, c.radius);
  const innerY = box.y + (box.h - contentH) / 2;
  let left: number;
  let top: number;
  if (wide) {
    drawBlock(c, { x: box.x + padX, y: box.y, w: figureW, h: box.h }, figure);
    left = box.x + padX + figureW + between + (gridRoomW - gridW) / 2;
    top = box.y + (box.h - (monthBand + gridH)) / 2 + monthBand;
  } else {
    drawBlock(
      c,
      { x: box.x + padX, y: innerY, w: box.w - padX * 2, h: figure.height },
      figure,
    );
    left = box.x + (box.w - gridW) / 2;
    top = innerY + figure.height + between + monthBand;
  }
  ctx.font = font(500, monthSize, TEXT);
  ctx.fillStyle = PANEL_MUTED;
  ctx.textAlign = flip("left");
  grid.months.forEach(({ col, label }) => {
    if (col > weeks - 2) return;
    ctx.fillText(label, mx(left + col * pitch), top - monthSize * 0.75);
  });
  grid.weeks.forEach((column, week) => {
    column.forEach((level, day) => {
      if (level < 0) return;
      roundRect(
        ctx,
        mx(left + week * pitch, cell),
        top + day * pitch,
        cell,
        cell,
        cell * 0.24,
      );
      ctx.fillStyle = HEAT[level];
      ctx.fill();
    });
  });
  ctx.textAlign = "left";
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
  locale: string,
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

  ctx.fillStyle = palette.ground;
  ctx.fillRect(0, 0, w, h);

  // A story sets its title right above the panel and centres the two in
  // the room it has; the other sizes keep the title at the top.
  let titleBaseline = layout.title.baseline;
  const titleGap = 56 * layout.k;
  const room = layout.grouped
    ? layout.region.h - layout.title.size - titleGap
    : layout.region.h;
  // The voice bars go down with the first panel placed, once its box is
  // known, so they can keep clear of it.
  let barsDrawn = false;
  const withBars = (box: Box): Box => {
    if (!barsDrawn) {
      barsDrawn = true;
      voiceBars(
        ctx,
        w,
        slide.kind,
        palette.bars,
        deck.week,
        layout.floor,
        layout.pad * 0.6,
        layout.grouped
          ? { ...box, y: box.y - layout.title.size - titleGap }
          : box,
      );
    }
    return box;
  };
  const placeBox = (height: number, full = false): Box => {
    const region = full
      ? { ...layout.region, w: w - layout.region.x * 2 }
      : layout.region;
    if (layout.grouped) {
      const block = layout.title.size + titleGap + height;
      const top = region.y + Math.max(0, (region.h - block) * 0.55);
      titleBaseline = top + layout.title.size * 0.8;
      return {
        x: region.x,
        y: top + layout.title.size + titleGap,
        w: region.w,
        h: Math.min(height, region.h - layout.title.size - titleGap),
      };
    }
    const fitted = Math.min(region.h, height);
    return {
      x: region.x,
      y: region.y + (region.h - fitted) / 2,
      w: region.w,
      h: fitted,
    };
  };
  const place = (height: number, full = false): Box =>
    withBars(placeBox(height, full));

  const c: Ctx = {
    ctx,
    palette,
    locale,
    format,
    k: layout.k,
    radius: layout.radius,
    room,
    place,
  };
  if (slide.kind === "highlights") highlightsCard(c, slide, layout.region);
  else if (slide.kind === "speed") speedCard(c, slide, layout.region);
  else if (slide.kind === "activity") activityCard(c, slide, layout.region);
  else figureCard(c, slide, layout.region);

  // fitSize leaves the title's font set.
  fitSize(
    ctx,
    slide.title,
    w - layout.pad * 2,
    layout.title.size,
    650,
    HEADING,
    -0.025,
  );
  ctx.fillStyle = palette.ink;
  ctx.textAlign = flip("left");
  ctx.fillText(slide.title, mx(layout.pad), titleBaseline);
  setTracking(ctx, 0, 1);

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
    ctx.font = font(500, size, TEXT);
    if (ctx.measureText(deck.tagline).width <= taglineRoom) {
      ctx.fillStyle = palette.tagline;
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
