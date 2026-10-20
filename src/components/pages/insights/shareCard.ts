import lockupLightUrl from "../../../../Logo/final-v2/png/lockup-h256.png";
import lockupDarkUrl from "../../../../Logo/final-v2/png/lockup-dark-h256.png";
import displayFontUrl from "@/assets/fonts/hubot-sans-latin-standard.woff2?url";
import digitsFontUrl from "@fontsource-variable/inter/files/inter-latin-wght-normal.woff2?url";
import {
  CARD_SITE,
  CARD_SIZES,
  type CardContent,
  type CardFact,
  type CardFormat,
  type CardLook,
} from "./shareContent";

/**
 * The share card: the Insights numbers as an image to post anywhere.
 *
 * It is drawn on a canvas rather than captured from the page, for two reasons.
 * The preview in the dialog and the saved file are then the same pixels, and
 * nothing depends on rasterising HTML, which WebKit refuses to export (an SVG
 * `foreignObject` taints the canvas there). Everything the card says is
 * decided in `shareContent.ts`; this file only lays it out.
 *
 * The look is the website's, not the app's: speakoflow.com's paper stock,
 * graphite ink and one teal, its display face for the headline, and the
 * recording pill as the centrepiece. The pill's bars are the user's own last
 * twelve weeks, one bar a day, so the one picture on the card is theirs.
 */

/* ───────────────────────────── drawing ───────────────────────────── */

const DISPLAY_FAMILY = "SpeakoFlow Display";
/** Inter's digits in front of the display face: its zero is drawn with a
 *  stroke through it, which reads as a theta at headline size. The website
 *  makes the same swap. */
const DIGITS_FAMILY = "SpeakoFlow Digits";
const DISPLAY = `"${DIGITS_FAMILY}", "${DISPLAY_FAMILY}", "Inter Variable", system-ui, sans-serif`;
const TEXT = `"Inter Variable", system-ui, -apple-system, "Segoe UI", sans-serif`;

interface Look {
  paint: (ctx: CanvasRenderingContext2D, w: number, h: number) => void;
  ink: string;
  muted: string;
  accent: string;
  lockup: "light" | "dark";
  pill: string;
  pillEdge: string | null;
  pillShadow: string;
  bar: string;
  barToday: string;
  panel: string;
  panelEdge: string | null;
  panelShadow: string | null;
  rule: string;
}

const radial = (
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  r: number,
  color: string,
) => {
  const gradient = ctx.createRadialGradient(x, y, 0, x, y, r);
  gradient.addColorStop(0, color);
  gradient.addColorStop(1, color.replace(/[\d.]+\)$/, "0)"));
  ctx.fillStyle = gradient;
  ctx.fillRect(0, 0, ctx.canvas.width, ctx.canvas.height);
};

/** The website's dotted stock, fading out toward the top and bottom. */
const dotGrid = (
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  rgb: string,
  alpha: number,
) => {
  const step = 30;
  for (let y = step / 2; y < h; y += step) {
    const fade = Math.sin((y / h) * Math.PI);
    ctx.fillStyle = `rgba(${rgb}, ${(alpha * fade).toFixed(3)})`;
    for (let x = step / 2; x < w; x += step) {
      ctx.beginPath();
      ctx.arc(x, y, 1.3, 0, Math.PI * 2);
      ctx.fill();
    }
  }
};

/**
 * The 2.0 launch art: broad bands of lavender and rose, each with a white
 * rim. They rise from the bottom edge and stay under the headline: on a tall
 * card they start below it, and on a wide one, whose headline sits on the
 * left, they climb toward the right, behind the pill and the figures.
 */
const contourBands = (ctx: CanvasRenderingContext2D, w: number, h: number) => {
  const wide = w > h * 1.3;
  const left = h * (wide ? 0.8 : 0.56);
  const right = h * (wide ? 0.3 : 0.48);
  const gap = Math.min(w, h) * 0.075;
  const tints = [
    "rgba(255, 255, 255, 0.24)",
    "rgba(190, 178, 246, 0.40)",
    "rgba(255, 255, 255, 0.18)",
    "rgba(240, 186, 222, 0.38)",
  ];
  const bands = 8;
  for (let i = 0; i < bands; i += 1) {
    const amp = w * (0.022 + 0.004 * i);
    const phase = i * 0.7;
    ctx.beginPath();
    ctx.moveTo(0, h);
    const steps = 64;
    for (let step = 0; step <= steps; step += 1) {
      const u = step / steps;
      const base = left + (right - left) * u + gap * i;
      const y =
        base -
        amp * Math.sin(u * Math.PI * 1.3 + phase) -
        amp * 0.3 * Math.sin(u * Math.PI * 3.1 + phase * 1.6);
      ctx.lineTo(u * w, y);
    }
    ctx.lineTo(w, h);
    ctx.closePath();
    ctx.fillStyle = tints[i % tints.length];
    ctx.fill();
    ctx.strokeStyle = "rgba(255, 255, 255, 0.85)";
    ctx.lineWidth = 2.5;
    ctx.stroke();
  }
};

const LOOKS: Record<CardLook, Look> = {
  paper: {
    paint: (ctx, w, h) => {
      ctx.fillStyle = "#f5f6f4";
      ctx.fillRect(0, 0, w, h);
      dotGrid(ctx, w, h, "19, 22, 24", 0.07);
      radial(
        ctx,
        w * 0.5,
        h * 0.62,
        Math.max(w, h) * 0.55,
        "rgba(20, 184, 166, 0.16)",
      );
      radial(ctx, w * 0.5, 0, w * 0.7, "rgba(255, 255, 255, 0.9)");
    },
    ink: "#131618",
    muted: "#5a5f64",
    accent: "#0b8a7b",
    lockup: "light",
    pill: "#131618",
    pillEdge: null,
    pillShadow: "rgba(19, 22, 24, 0.32)",
    bar: "#ffffff",
    barToday: "#5eead4",
    panel: "#ffffff",
    panelEdge: "rgba(20, 184, 166, 0.45)",
    panelShadow: "rgba(13, 148, 136, 0.2)",
    rule: "rgba(19, 22, 24, 0.09)",
  },
  graphite: {
    paint: (ctx, w, h) => {
      ctx.fillStyle = "#0f1315";
      ctx.fillRect(0, 0, w, h);
      ctx.strokeStyle = "rgba(255, 255, 255, 0.035)";
      ctx.lineWidth = 1;
      for (let x = 0; x <= w; x += 96) {
        ctx.beginPath();
        ctx.moveTo(x + 0.5, 0);
        ctx.lineTo(x + 0.5, h);
        ctx.stroke();
      }
      for (let y = 0; y <= h; y += 96) {
        ctx.beginPath();
        ctx.moveTo(0, y + 0.5);
        ctx.lineTo(w, y + 0.5);
        ctx.stroke();
      }
      radial(
        ctx,
        w * 0.9,
        h * 0.08,
        Math.max(w, h) * 0.5,
        "rgba(12, 111, 101, 0.3)",
      );
      radial(
        ctx,
        w * 0.5,
        h * 0.66,
        Math.max(w, h) * 0.45,
        "rgba(20, 184, 166, 0.14)",
      );
    },
    ink: "#eef1f0",
    muted: "#a7b0b0",
    accent: "#5fe0cd",
    lockup: "dark",
    pill: "#1a2023",
    pillEdge: "rgba(255, 255, 255, 0.09)",
    pillShadow: "rgba(0, 0, 0, 0.5)",
    bar: "#ffffff",
    barToday: "#5fe0cd",
    panel: "rgba(255, 255, 255, 0.045)",
    panelEdge: "rgba(255, 255, 255, 0.09)",
    panelShadow: null,
    rule: "rgba(255, 255, 255, 0.09)",
  },
  mint: {
    paint: (ctx, w, h) => {
      const base = ctx.createLinearGradient(0, 0, 0, h);
      base.addColorStop(0, "#fbfcfa");
      base.addColorStop(1, "#f2f8f3");
      ctx.fillStyle = base;
      ctx.fillRect(0, 0, w, h);
      const r = Math.max(w, h);
      radial(ctx, w * 0.08, h * 0.12, r * 0.5, "rgba(213, 244, 160, 0.75)");
      radial(ctx, w * 0.92, h * 0.06, r * 0.45, "rgba(176, 238, 246, 0.85)");
      radial(ctx, w * 0.5, h * 1.0, r * 0.62, "rgba(74, 222, 128, 0.5)");
      radial(ctx, w * 0.04, h * 0.82, r * 0.42, "rgba(94, 214, 180, 0.6)");
      radial(ctx, w * 0.98, h * 0.78, r * 0.38, "rgba(134, 230, 160, 0.55)");
      radial(ctx, w * 0.5, h * 0.4, r * 0.4, "rgba(255, 255, 255, 0.55)");
    },
    ink: "#0f1d1a",
    muted: "#3f514b",
    accent: "#0a7a70",
    lockup: "light",
    pill: "#131618",
    pillEdge: null,
    pillShadow: "rgba(10, 60, 50, 0.3)",
    bar: "#ffffff",
    barToday: "#5eead4",
    panel: "rgba(255, 255, 255, 0.8)",
    panelEdge: "rgba(15, 29, 26, 0.06)",
    panelShadow: "rgba(10, 60, 50, 0.14)",
    rule: "rgba(15, 29, 26, 0.1)",
  },
  dusk: {
    paint: (ctx, w, h) => {
      const base = ctx.createLinearGradient(0, 0, w, h);
      base.addColorStop(0, "#c3cdf5");
      base.addColorStop(0.55, "#dcd3f3");
      base.addColorStop(1, "#f4d3e3");
      ctx.fillStyle = base;
      ctx.fillRect(0, 0, w, h);
      contourBands(ctx, w, h);
      radial(ctx, w * 0.5, 0, w * 0.8, "rgba(255, 255, 255, 0.55)");
    },
    ink: "#1b1d33",
    muted: "#474a68",
    accent: "#4f46d8",
    lockup: "light",
    pill: "#1b1d33",
    pillEdge: null,
    pillShadow: "rgba(40, 30, 90, 0.32)",
    bar: "#ffffff",
    barToday: "#c4b5fd",
    panel: "rgba(255, 255, 255, 0.66)",
    panelEdge: "rgba(255, 255, 255, 0.8)",
    panelShadow: "rgba(40, 30, 90, 0.12)",
    rule: "rgba(27, 29, 51, 0.1)",
  },
};

/** A swatch for the look picker: the look's ground and accent. */
export const LOOK_SWATCHES: Record<
  CardLook,
  { ground: string; accent: string }
> = {
  paper: { ground: "#f5f6f4", accent: "#131618" },
  graphite: {
    ground: "radial-gradient(circle at 88% 12%, #12332f, #0f1315 70%)",
    accent: "#5fe0cd",
  },
  mint: {
    ground: "linear-gradient(135deg,#d5f4a0,#b0eef6 55%,#86e6a0)",
    accent: "#0a7a70",
  },
  dusk: {
    ground: "linear-gradient(135deg,#c3cdf5,#dcd3f3 55%,#f4d3e3)",
    accent: "#4f46d8",
  },
};

interface Assets {
  lockupLight: HTMLImageElement;
  lockupDark: HTMLImageElement;
}

let assetsPromise: Promise<Assets> | null = null;

const loadImage = (src: string) =>
  new Promise<HTMLImageElement>((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error(`Could not load ${src}`));
    image.src = src;
  });

/** Fonts and logos, loaded once. A face that fails to load falls back to
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
    assetsPromise = Promise.all([
      fonts,
      loadImage(lockupLightUrl),
      loadImage(lockupDarkUrl),
    ])
      .then(([, lockupLight, lockupDark]) => ({ lockupLight, lockupDark }))
      .catch((error) => {
        assetsPromise = null;
        throw error;
      });
  }
  return assetsPromise;
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

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * A right-to-left language reads the card from the right, so the card is
 * drawn as the mirror image of its left-to-right layout: text aligned right,
 * the logo at the top right, and the voiceprint running from today on the
 * left. Layout is written left to right; these two turn it around. Set by
 * `drawCard` for the card being drawn.
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

/** The headline: the figure, then the sentence it starts. Returns the height
 *  it takes at `scale`, drawing only when asked. */
const headline = (
  ctx: CanvasRenderingContext2D,
  content: CardContent,
  look: Look,
  box: Box,
  sizes: { value: number; words: number },
  draw: boolean,
): number => {
  const valueSize = sizes.value;
  const wordsSize = sizes.words;
  ctx.textAlign = flip("left");
  ctx.textBaseline = "alphabetic";

  ctx.font = font(720, valueSize, DISPLAY);
  setTracking(ctx, -0.045, valueSize);
  let y = box.y + valueSize * 0.78;
  if (draw) {
    ctx.fillStyle = look.ink;
    ctx.fillText(content.value, mx(box.x - valueSize * 0.04), y);
  }

  ctx.font = font(700, wordsSize, DISPLAY);
  setTracking(ctx, -0.035, wordsSize);
  const leadLines = wrap(ctx, content.lead, box.w);
  const accentLines = wrap(ctx, content.accent, box.w);
  y += valueSize * 0.14;
  const lineHeight = wordsSize * 1.0;
  for (const [lines, color] of [
    [leadLines, look.ink],
    [accentLines, look.accent],
  ] as const) {
    for (const line of lines) {
      y += lineHeight;
      if (draw) {
        ctx.fillStyle = color;
        ctx.fillText(line, mx(box.x), y);
      }
    }
  }
  setTracking(ctx, 0, 1);
  ctx.textAlign = "left";
  return y + wordsSize * 0.24 - box.y;
};

/** Whether a line of the headline's sentence would hold one word alone. */
const orphaned = (
  ctx: CanvasRenderingContext2D,
  content: CardContent,
  width: number,
  words: number,
): boolean => {
  ctx.font = font(700, words, DISPLAY);
  setTracking(ctx, -0.035, words);
  const alone = (text: string) => {
    const lines = wrap(ctx, text, width);
    return lines.length > 1 && !/\s/.test(lines[lines.length - 1].trim());
  };
  const result = alone(content.lead) || alone(content.accent);
  setTracking(ctx, 0, 1);
  return result;
};

/** Fit the headline into `box`, shrinking from `sizes` until it does, then
 *  draw it with `bias` of the spare room above it. */
const fitHeadline = (
  ctx: CanvasRenderingContext2D,
  content: CardContent,
  look: Look,
  box: Box,
  sizes: { value: number; words: number },
  bias: number,
) => {
  // The largest size that fits, unless it leaves one word alone on a line
  // and a size within a fifth of it does not.
  let fits: number | null = null;
  let chosen: number | null = null;
  for (let scale = 1; scale > 0.45; scale -= 0.02) {
    const scaled = { value: sizes.value * scale, words: sizes.words * scale };
    ctx.font = font(720, scaled.value, DISPLAY);
    setTracking(ctx, -0.045, scaled.value);
    const fitsWide = ctx.measureText(content.value).width <= box.w;
    const height = headline(ctx, content, look, box, scaled, false);
    if (!fitsWide || height > box.h) continue;
    fits ??= scale;
    if (!orphaned(ctx, content, box.w, scaled.words)) {
      chosen = scale;
      break;
    }
    if (scale < fits * 0.8) break;
  }
  const scale = chosen ?? fits ?? 0.45;
  const scaled = { value: sizes.value * scale, words: sizes.words * scale };
  const height = headline(ctx, content, look, box, scaled, false);
  const offset = Math.max(0, (box.h - height) * bias);
  headline(ctx, content, look, { ...box, y: box.y + offset }, scaled, true);
};

/** The recording pill, with a bar per day of the voiceprint. */
const voiceprint = (
  ctx: CanvasRenderingContext2D,
  content: CardContent,
  look: Look,
  box: Box,
) => {
  ctx.save();
  ctx.shadowColor = look.pillShadow;
  ctx.shadowBlur = box.h * 0.45;
  ctx.shadowOffsetY = box.h * 0.16;
  roundRect(ctx, mx(box.x, box.w), box.y, box.w, box.h, box.h / 2);
  ctx.fillStyle = look.pill;
  ctx.fill();
  ctx.restore();
  if (look.pillEdge) {
    roundRect(
      ctx,
      mx(box.x, box.w) + 0.5,
      box.y + 0.5,
      box.w - 1,
      box.h - 1,
      box.h / 2,
    );
    ctx.strokeStyle = look.pillEdge;
    ctx.lineWidth = 1.5;
    ctx.stroke();
  }

  const bars = content.bars;
  const inset = box.h * 0.42;
  const pitch = (box.w - inset * 2) / bars.length;
  const width = Math.max(2, pitch * 0.5);
  const mid = box.y + box.h / 2;
  const tallest = box.h * 0.62;
  const max = Math.max(1, ...bars);
  bars.forEach((value, index) => {
    const x = mx(box.x + inset + pitch * index + (pitch - width) / 2, width);
    // Older days a little quieter, so the line reads toward today.
    const age = index / Math.max(1, bars.length - 1);
    const today = index === bars.length - 1;
    ctx.fillStyle = today && value > 0 ? look.barToday : look.bar;
    if (value <= 0) {
      ctx.globalAlpha = 0.28;
      ctx.beginPath();
      ctx.arc(x + width / 2, mid, width / 2, 0, Math.PI * 2);
      ctx.fill();
    } else {
      ctx.globalAlpha = 0.55 + 0.45 * age;
      const height = tallest * (0.16 + 0.84 * Math.sqrt(value / max));
      roundRect(ctx, x, mid - height / 2, width, height, width / 2);
      ctx.fill();
    }
  });
  ctx.globalAlpha = 1;
};

const caption = (
  ctx: CanvasRenderingContext2D,
  text: string,
  look: Look,
  x: number,
  y: number,
  size: number,
  align: CanvasTextAlign = "left",
) => {
  ctx.font = font(500, size, TEXT);
  ctx.textAlign = flip(align);
  ctx.fillStyle = look.muted;
  ctx.fillText(text, mx(x), y);
  ctx.textAlign = "left";
};

const panel = (
  ctx: CanvasRenderingContext2D,
  look: Look,
  box: Box,
  r: number,
) => {
  ctx.save();
  if (look.panelShadow) {
    ctx.shadowColor = look.panelShadow;
    ctx.shadowBlur = 48;
    ctx.shadowOffsetY = 18;
  }
  roundRect(ctx, mx(box.x, box.w), box.y, box.w, box.h, r);
  ctx.fillStyle = look.panel;
  ctx.fill();
  ctx.restore();
  if (look.panelEdge) {
    roundRect(
      ctx,
      mx(box.x, box.w) + 0.5,
      box.y + 0.5,
      box.w - 1,
      box.h - 1,
      r,
    );
    ctx.strokeStyle = look.panelEdge;
    ctx.lineWidth = 1.5;
    ctx.stroke();
  }
};

/** A label at `size`, shrunk by up to a quarter until it takes at most two
 *  lines of `width`, then cut to two. */
const fitLabel = (
  ctx: CanvasRenderingContext2D,
  label: string,
  width: number,
  size: number,
): { size: number; lines: string[] } => {
  let fitted = size;
  for (;;) {
    ctx.font = font(500, fitted, TEXT);
    const lines = wrap(ctx, label, width);
    if (lines.length <= 2 || fitted <= size * 0.75) {
      return { size: fitted, lines: lines.slice(0, 2) };
    }
    fitted -= 1;
  }
};

/** The value set at `size`, shrunk until it is no wider than `width`. */
const fitValue = (
  ctx: CanvasRenderingContext2D,
  value: string,
  width: number,
  size: number,
): number => {
  let fitted = size;
  for (;;) {
    ctx.font = font(720, fitted, DISPLAY);
    setTracking(ctx, -0.02, fitted);
    if (ctx.measureText(value).width <= width || fitted <= 18) return fitted;
    fitted -= 2;
  }
};

interface FactSizes {
  value: number;
  label: number;
  /** Inset from the panel's sides. */
  pad: number;
  /** Inset from a cell's top and bottom; `pad` when left out. */
  padY?: number;
  radius: number;
}

/**
 * One figure: the value, and its label under it in up to two lines. Every
 * cell in a row is centred as if it had `rowLines` lines of label, so the
 * values in a row line up whichever of their labels wraps.
 */
const fact = (
  ctx: CanvasRenderingContext2D,
  item: CardFact,
  look: Look,
  box: Box,
  sizes: FactSizes,
  rowLines: number,
) => {
  const label = fitLabel(ctx, item.label, box.w, sizes.label);
  const valueSize = fitValue(ctx, item.value, box.w, sizes.value);
  const height =
    valueSize * 0.76 +
    label.size * 1.55 +
    (rowLines - 1) * label.size * 1.25 +
    label.size * 0.25;
  const valueY = box.y + Math.max(0, (box.h - height) / 2) + valueSize * 0.76;
  ctx.fillStyle = look.ink;
  ctx.textAlign = flip("left");
  ctx.fillText(item.value, mx(box.x), valueY);
  setTracking(ctx, 0, 1);

  ctx.font = font(500, label.size, TEXT);
  ctx.fillStyle = look.muted;
  label.lines.forEach((line, index) => {
    ctx.fillText(
      line,
      mx(box.x),
      valueY + label.size * 1.55 + index * label.size * 1.25,
    );
  });
  ctx.textAlign = "left";
};

/**
 * One figure on a line of its own: the value at the start and its label at
 * the end, on one baseline, like a line of a receipt. For a single column,
 * where a label under each value would stack the panel twice as tall.
 */
const ledgerLine = (
  ctx: CanvasRenderingContext2D,
  item: CardFact,
  look: Look,
  box: Box,
  sizes: FactSizes,
) => {
  const valueSize = fitValue(ctx, item.value, box.w * 0.55, sizes.value);
  const valueWidth = ctx.measureText(item.value).width;
  const room = box.w - valueWidth - sizes.label * 1.2;
  const label = fitLabel(ctx, item.label, room, sizes.label);
  const baseline = box.y + box.h / 2 + valueSize * 0.36;
  ctx.font = font(720, valueSize, DISPLAY);
  setTracking(ctx, -0.02, valueSize);
  ctx.fillStyle = look.ink;
  ctx.textAlign = flip("left");
  ctx.fillText(item.value, mx(box.x), baseline);
  setTracking(ctx, 0, 1);

  ctx.font = font(500, label.size, TEXT);
  ctx.fillStyle = look.muted;
  ctx.textAlign = flip("right");
  const step = label.size * 1.2;
  const first = baseline - (label.lines.length - 1) * step;
  label.lines.forEach((line, index) => {
    ctx.fillText(line, mx(box.x + box.w), first + index * step);
  });
  ctx.textAlign = "left";
};

/** Figures in a grid of `columns` divided by hairlines, inside one panel, or
 *  in one column as a list of lines. No figures, no panel: an empty white box
 *  says something is missing. */
const factGrid = (
  ctx: CanvasRenderingContext2D,
  facts: CardFact[],
  look: Look,
  box: Box,
  columns: number,
  sizes: FactSizes,
) => {
  if (facts.length === 0) return;
  panel(ctx, look, box, sizes.radius);
  const padY = sizes.padY ?? sizes.pad;
  const rows = Math.ceil(facts.length / columns);
  const cellW = box.w / columns;
  const cellH = box.h / rows;
  for (let row = 1; row < rows; row += 1) {
    ctx.fillStyle = look.rule;
    ctx.fillRect(
      mx(box.x + sizes.pad, box.w - sizes.pad * 2),
      box.y + cellH * row,
      box.w - sizes.pad * 2,
      1.5,
    );
  }
  for (let col = 1; col < columns; col += 1) {
    ctx.fillStyle = look.rule;
    ctx.fillRect(
      mx(box.x + cellW * col, 1.5),
      box.y + padY,
      1.5,
      box.h - padY * 2,
    );
  }
  const cell = (index: number): Box => ({
    x: box.x + cellW * (index % columns) + sizes.pad,
    y: box.y + cellH * Math.floor(index / columns) + padY,
    w: cellW - sizes.pad * 2,
    h: cellH - padY * 2,
  });
  if (columns === 1) {
    facts.forEach((item, index) =>
      ledgerLine(ctx, item, look, cell(index), sizes),
    );
    return;
  }
  const lines = facts.map(
    (item, index) =>
      fitLabel(ctx, item.label, cell(index).w, sizes.label).lines.length,
  );
  facts.forEach((item, index) => {
    const row = Math.floor(index / columns);
    const rowLines = Math.max(
      ...lines.slice(row * columns, row * columns + columns),
    );
    fact(ctx, item, look, cell(index), sizes, rowLines);
  });
};

/** The logo at the top left and the period at the top right. */
const header = (
  ctx: CanvasRenderingContext2D,
  content: CardContent,
  look: Look,
  assets: Assets,
  x: number,
  y: number,
  right: number,
  logoHeight: number,
  textSize: number,
) => {
  const logo = look.lockup === "dark" ? assets.lockupDark : assets.lockupLight;
  const logoWidth = (logo.naturalWidth / logo.naturalHeight) * logoHeight;
  ctx.drawImage(logo, mx(x, logoWidth), y, logoWidth, logoHeight);
  if (content.period) {
    caption(
      ctx,
      content.period,
      look,
      right,
      y + logoHeight / 2 + textSize * 0.36,
      textSize,
      "right",
    );
  }
};

/** The address and what the app is, along the bottom edge. */
const footer = (
  ctx: CanvasRenderingContext2D,
  content: CardContent,
  look: Look,
  x: number,
  baseline: number,
  right: number,
  size: number,
) => {
  ctx.font = font(650, size, TEXT);
  ctx.fillStyle = look.ink;
  ctx.textAlign = flip("left");
  ctx.fillText(CARD_SITE, mx(x), baseline);
  ctx.textAlign = "left";
  const siteWidth = ctx.measureText(CARD_SITE).width;
  // The tagline gives way before the address does: smaller first, and left
  // out only when even that would run into it.
  const room = right - x - siteWidth - size * 1.5;
  for (let tagSize = size * 0.9; tagSize >= size * 0.7; tagSize -= 1) {
    ctx.font = font(500, tagSize, TEXT);
    if (ctx.measureText(content.tagline).width <= room) {
      caption(ctx, content.tagline, look, right, baseline, tagSize, "right");
      return;
    }
  }
};

/** Draw the card at its full size onto `canvas`, mirrored for a
 *  right-to-left language. Assets must be loaded. */
export const drawCard = (
  canvas: HTMLCanvasElement,
  content: CardContent,
  format: CardFormat,
  lookName: CardLook,
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
  const look = LOOKS[lookName];
  ctx.clearRect(0, 0, w, h);
  look.paint(ctx, w, h);

  if (format === "square") {
    const pad = 76;
    const logo = 54;
    header(ctx, content, look, assets, pad, pad, w - pad, logo, 28);
    const footBase = h - pad + 6;
    footer(ctx, content, look, pad, footBase, w - pad, 31);
    const shown = content.facts.slice(0, 3);
    const facts = { x: pad, y: footBase - 62 - 192, w: w - pad * 2, h: 192 };
    factGrid(ctx, shown, look, facts, Math.max(1, shown.length), {
      value: 58,
      label: 27,
      pad: 30,
      radius: 32,
    });
    const captionY = (shown.length ? facts.y : footBase - 40) - 40;
    caption(ctx, content.barsCaption, look, pad + 4, captionY, 26);
    const pill = {
      x: pad,
      y: captionY - 26 - 30 - 128,
      w: w - pad * 2,
      h: 128,
    };
    voiceprint(ctx, content, look, pill);
    const top = pad + logo + 60;
    fitHeadline(
      ctx,
      content,
      look,
      { x: pad, y: top, w: w - pad * 2, h: pill.y - 52 - top },
      { value: 192, words: 76 },
      0,
    );
  } else if (format === "story") {
    // Instagram and Facebook draw their own bars over about the top and
    // bottom 250 pixels of a story, so the logo and the address stay inside.
    const pad = 88;
    const safeTop = 250;
    const safeBottom = h - 260;
    const logo = 62;
    header(ctx, content, look, assets, pad, safeTop, w - pad, logo, 32);
    footer(ctx, content, look, pad, safeBottom, w - pad, 36);
    const shown = content.facts.slice(0, 4);
    const columns = shown.length === 4 ? 2 : 1;
    const factsH = shown.length === 4 ? 340 : shown.length * 108 + 24;
    const facts = {
      x: pad,
      y: safeBottom - 70 - factsH,
      w: w - pad * 2,
      h: factsH,
    };
    factGrid(ctx, shown, look, facts, columns, {
      value: 74,
      label: 34,
      pad: 40,
      padY: columns === 1 ? 12 : 40,
      radius: 40,
    });
    const captionY = (shown.length ? facts.y : safeBottom - 50) - 48;
    caption(ctx, content.barsCaption, look, pad + 4, captionY, 32);
    const pill = {
      x: pad,
      y: captionY - 32 - 34 - 190,
      w: w - pad * 2,
      h: 190,
    };
    voiceprint(ctx, content, look, pill);
    const top = safeTop + logo + 80;
    fitHeadline(
      ctx,
      content,
      look,
      { x: pad, y: top, w: w - pad * 2, h: pill.y - 80 - top },
      { value: 280, words: 112 },
      0.42,
    );
  } else {
    const pad = 80;
    const split = 860;
    const logo = 54;
    header(ctx, content, look, assets, pad, pad, w - pad, logo, 28);
    const footBase = h - pad + 6;
    footer(ctx, content, look, pad, footBase, w - pad, 31);
    const right = { x: split, w: w - pad - split };
    const pill = { x: right.x, y: pad + logo + 66, w: right.w, h: 120 };
    voiceprint(ctx, content, look, pill);
    caption(
      ctx,
      content.barsCaption,
      look,
      right.x + 4,
      pill.y + pill.h + 46,
      26,
    );
    const shown = content.facts.slice(0, 4);
    const factsY = pill.y + pill.h + 82;
    factGrid(
      ctx,
      shown,
      look,
      { x: right.x, y: factsY, w: right.w, h: footBase - 60 - factsY },
      shown.length === 4 ? 2 : 1,
      {
        value: 54,
        label: 26,
        pad: 30,
        padY: shown.length === 4 ? 30 : 8,
        radius: 30,
      },
    );
    const top = pad + logo + 66;
    fitHeadline(
      ctx,
      content,
      look,
      { x: pad, y: top, w: split - pad - 70, h: footBase - 70 - top },
      { value: 200, words: 80 },
      0.5,
    );
  }
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
