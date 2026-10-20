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

/** The 2.0 launch art: layered bands of lavender with a white edge. */
const contourBands = (ctx: CanvasRenderingContext2D, w: number, h: number) => {
  const bands = 9;
  const tints = [
    "rgba(255, 255, 255, 0.16)",
    "rgba(206, 196, 246, 0.30)",
    "rgba(255, 255, 255, 0.12)",
    "rgba(240, 196, 226, 0.28)",
  ];
  for (let i = 0; i < bands; i += 1) {
    const base = h * (0.34 + i * 0.085);
    const amp = h * (0.05 + 0.012 * i);
    const phase = i * 0.55;
    ctx.beginPath();
    ctx.moveTo(0, h);
    const steps = 48;
    for (let s = 0; s <= steps; s += 1) {
      const x = (s / steps) * w;
      const u = s / steps;
      const y =
        base -
        amp * Math.sin(u * Math.PI * 1.15 + phase) -
        amp * 0.35 * Math.sin(u * Math.PI * 2.6 + phase * 1.7);
      ctx.lineTo(x, y);
    }
    ctx.lineTo(w, h);
    ctx.closePath();
    ctx.fillStyle = tints[i % tints.length];
    ctx.fill();
    ctx.strokeStyle = "rgba(255, 255, 255, 0.75)";
    ctx.lineWidth = 2;
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
    panelEdge: "rgba(19, 22, 24, 0.07)",
    panelShadow: "rgba(19, 22, 24, 0.14)",
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
        w * 0.88,
        h * 0.1,
        Math.max(w, h) * 0.75,
        "rgba(12, 111, 101, 0.55)",
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
  graphite: { ground: "#0f1315", accent: "#5fe0cd" },
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
  ctx.textAlign = "left";
  ctx.textBaseline = "alphabetic";

  ctx.font = font(720, valueSize, DISPLAY);
  setTracking(ctx, -0.045, valueSize);
  let y = box.y + valueSize * 0.78;
  if (draw) {
    ctx.fillStyle = look.ink;
    ctx.fillText(content.value, box.x - valueSize * 0.04, y);
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
        ctx.fillText(line, box.x, y);
      }
    }
  }
  setTracking(ctx, 0, 1);
  return y + wordsSize * 0.24 - box.y;
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
  let scale = 1;
  let height = 0;
  for (; scale > 0.45; scale -= 0.04) {
    const scaled = { value: sizes.value * scale, words: sizes.words * scale };
    ctx.font = font(720, scaled.value, DISPLAY);
    setTracking(ctx, -0.045, scaled.value);
    const fitsWide = ctx.measureText(content.value).width <= box.w;
    height = headline(ctx, content, look, box, scaled, false);
    if (fitsWide && height <= box.h) break;
  }
  const offset = Math.max(0, (box.h - height) * bias);
  headline(
    ctx,
    content,
    look,
    { ...box, y: box.y + offset },
    { value: sizes.value * scale, words: sizes.words * scale },
    true,
  );
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
  roundRect(ctx, box.x, box.y, box.w, box.h, box.h / 2);
  ctx.fillStyle = look.pill;
  ctx.fill();
  ctx.restore();
  if (look.pillEdge) {
    roundRect(ctx, box.x + 0.5, box.y + 0.5, box.w - 1, box.h - 1, box.h / 2);
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
    const x = box.x + inset + pitch * index + (pitch - width) / 2;
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
  ctx.textAlign = align;
  ctx.fillStyle = look.muted;
  ctx.fillText(text, x, y);
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
  roundRect(ctx, box.x, box.y, box.w, box.h, r);
  ctx.fillStyle = look.panel;
  ctx.fill();
  ctx.restore();
  if (look.panelEdge) {
    roundRect(ctx, box.x + 0.5, box.y + 0.5, box.w - 1, box.h - 1, r);
    ctx.strokeStyle = look.panelEdge;
    ctx.lineWidth = 1.5;
    ctx.stroke();
  }
};

/** One figure: the value, and its label under it in up to two lines,
 *  centred in `box` from top to bottom. */
const fact = (
  ctx: CanvasRenderingContext2D,
  item: CardFact,
  look: Look,
  box: Box,
  sizes: { value: number; label: number },
) => {
  ctx.font = font(500, sizes.label, TEXT);
  const lines = wrap(ctx, item.label, box.w).slice(0, 2);

  let valueSize = sizes.value;
  const fitValue = () => {
    ctx.font = font(720, valueSize, DISPLAY);
    setTracking(ctx, -0.02, valueSize);
  };
  fitValue();
  while (ctx.measureText(item.value).width > box.w && valueSize > 18) {
    valueSize -= 2;
    fitValue();
  }
  const height =
    valueSize * 0.76 +
    sizes.label * 1.55 +
    (lines.length - 1) * sizes.label * 1.25 +
    sizes.label * 0.25;
  const valueY = box.y + Math.max(0, (box.h - height) / 2) + valueSize * 0.76;
  ctx.fillStyle = look.ink;
  ctx.textAlign = "left";
  ctx.fillText(item.value, box.x, valueY);
  setTracking(ctx, 0, 1);

  ctx.font = font(500, sizes.label, TEXT);
  ctx.fillStyle = look.muted;
  lines.forEach((line, index) => {
    ctx.fillText(
      line,
      box.x,
      valueY + sizes.label * 1.55 + index * sizes.label * 1.25,
    );
  });
};

/** Figures in a row of columns divided by hairlines, inside one panel. */
const factRow = (
  ctx: CanvasRenderingContext2D,
  facts: CardFact[],
  look: Look,
  box: Box,
  sizes: { value: number; label: number; pad: number; radius: number },
) => {
  panel(ctx, look, box, sizes.radius);
  const columns = facts.length;
  const width = box.w / columns;
  facts.forEach((item, index) => {
    const x = box.x + width * index;
    if (index > 0) {
      ctx.fillStyle = look.rule;
      ctx.fillRect(x, box.y + sizes.pad, 1.5, box.h - sizes.pad * 2);
    }
    fact(
      ctx,
      item,
      look,
      {
        x: x + sizes.pad,
        y: box.y + sizes.pad,
        w: width - sizes.pad * 2,
        h: box.h - sizes.pad * 2,
      },
      sizes,
    );
  });
};

/** Figures in a grid of `columns`, inside one panel. */
const factGrid = (
  ctx: CanvasRenderingContext2D,
  facts: CardFact[],
  look: Look,
  box: Box,
  columns: number,
  sizes: { value: number; label: number; pad: number; radius: number },
) => {
  panel(ctx, look, box, sizes.radius);
  const rows = Math.ceil(facts.length / columns);
  const cellW = box.w / columns;
  const cellH = box.h / rows;
  for (let row = 1; row < rows; row += 1) {
    ctx.fillStyle = look.rule;
    ctx.fillRect(
      box.x + sizes.pad,
      box.y + cellH * row,
      box.w - sizes.pad * 2,
      1.5,
    );
  }
  for (let col = 1; col < columns; col += 1) {
    ctx.fillStyle = look.rule;
    ctx.fillRect(
      box.x + cellW * col,
      box.y + sizes.pad,
      1.5,
      box.h - sizes.pad * 2,
    );
  }
  facts.forEach((item, index) => {
    const col = index % columns;
    const row = Math.floor(index / columns);
    fact(
      ctx,
      item,
      look,
      {
        x: box.x + cellW * col + sizes.pad,
        y: box.y + cellH * row + sizes.pad,
        w: cellW - sizes.pad * 2,
        h: cellH - sizes.pad * 2,
      },
      sizes,
    );
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
  ctx.drawImage(logo, x, y, logoWidth, logoHeight);
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
  ctx.textAlign = "left";
  ctx.fillText(CARD_SITE, x, baseline);
  const siteWidth = ctx.measureText(CARD_SITE).width;
  ctx.font = font(500, size * 0.9, TEXT);
  if (
    ctx.measureText(content.tagline).width <
    right - x - siteWidth - size * 2
  ) {
    caption(ctx, content.tagline, look, right, baseline, size * 0.9, "right");
  }
};

/** Draw the card at its full size onto `canvas`. Assets must be loaded. */
export const drawCard = (
  canvas: HTMLCanvasElement,
  content: CardContent,
  format: CardFormat,
  lookName: CardLook,
  assets: Assets,
) => {
  const { width: w, height: h } = CARD_SIZES[format];
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const look = LOOKS[lookName];
  ctx.clearRect(0, 0, w, h);
  look.paint(ctx, w, h);

  if (format === "square") {
    const pad = 76;
    header(ctx, content, look, assets, pad, pad, w - pad, 50, 24);
    const footBase = h - pad + 6;
    footer(ctx, content, look, pad, footBase, w - pad, 25);
    const facts = { x: pad, y: footBase - 58 - 154, w: w - pad * 2, h: 154 };
    factRow(ctx, content.facts.slice(0, 3), look, facts, {
      value: 50,
      label: 21,
      pad: 30,
      radius: 30,
    });
    const captionY = facts.y - 38;
    caption(ctx, content.barsCaption, look, pad + 4, captionY, 21);
    const pill = { x: pad, y: captionY - 30 - 132, w: w - pad * 2, h: 132 };
    voiceprint(ctx, content, look, pill);
    fitHeadline(
      ctx,
      content,
      look,
      {
        x: pad,
        y: pad + 50 + 64,
        w: w - pad * 2,
        h: pill.y - 52 - (pad + 50 + 64),
      },
      { value: 192, words: 76 },
      0,
    );
  } else if (format === "story") {
    const pad = 88;
    header(ctx, content, look, assets, pad, pad + 8, w - pad, 58, 28);
    const footBase = h - pad;
    footer(ctx, content, look, pad, footBase, w - pad, 29);
    const facts = { x: pad, y: footBase - 70 - 380, w: w - pad * 2, h: 380 };
    factGrid(ctx, content.facts.slice(0, 4), look, facts, 2, {
      value: 66,
      label: 26,
      pad: 44,
      radius: 40,
    });
    const captionY = facts.y - 50;
    caption(ctx, content.barsCaption, look, pad + 4, captionY, 26);
    const pill = { x: pad, y: captionY - 38 - 200, w: w - pad * 2, h: 200 };
    voiceprint(ctx, content, look, pill);
    fitHeadline(
      ctx,
      content,
      look,
      {
        x: pad,
        y: pad + 66 + 120,
        w: w - pad * 2,
        h: pill.y - 90 - (pad + 66 + 120),
      },
      { value: 280, words: 112 },
      0.42,
    );
  } else {
    const pad = 80;
    const split = 880;
    header(ctx, content, look, assets, pad, pad, w - pad, 50, 24);
    const footBase = h - pad + 6;
    footer(ctx, content, look, pad, footBase, w - pad, 25);
    const right = { x: split, w: w - pad - split };
    const pill = { x: right.x, y: pad + 50 + 70, w: right.w, h: 118 };
    voiceprint(ctx, content, look, pill);
    caption(
      ctx,
      content.barsCaption,
      look,
      right.x + 4,
      pill.y + pill.h + 44,
      21,
    );
    const facts = {
      x: right.x,
      y: pill.y + pill.h + 76,
      w: right.w,
      h: footBase - 56 - (pill.y + pill.h + 76),
    };
    factGrid(ctx, content.facts.slice(0, 4), look, facts, 2, {
      value: 48,
      label: 20,
      pad: 30,
      radius: 30,
    });
    fitHeadline(
      ctx,
      content,
      look,
      {
        x: pad,
        y: pad + 50 + 70,
        w: split - pad - 80,
        h: footBase - 70 - (pad + 50 + 70),
      },
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
