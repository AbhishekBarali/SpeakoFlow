// The Linux tray icons (issue #76).
//
// A Linux panel draws its own icons as one-colour glyphs, and the colour app
// tile stood out among them. So by default the Linux tray shows a white glyph
// of the app tile: the rounded square with the logo's five bars cut out of it.
// It is the app icon reduced to one colour, which is how a branded app's panel
// icon usually looks (a silhouette with knockouts), and it cannot be mistaken
// for the transcribing glyph, which is bars on their own.
//
// The bars are wider and taller than the logo's because a
// 16px panel blurs the logo's thin cut-outs into a grey smear. Sizes are on
// the 64px canvas every tray PNG here uses; the panel scales it down.
//
// A panel can be light, and Linux gives the app no reliable way to find out,
// so "Color tray icon" in Settings switches back to the colour tile. Its state
// glyphs used to be Handy's pink, which is hard to see on a light panel (about
// 1.9:1); they are now the white glyphs' shapes in a brand teal that reads on
// both (3.5:1 on a light panel, 5:1 on black).
//
// Run from the Logo folder: node tray-glyphs.mjs

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const REPO = fileURLToPath(new URL("..", import.meta.url));
const RES = (name) => path.join(REPO, "src-tauri", "resources", name);

export const SIZE = 64;
// The white of the existing state glyphs (tray_recording.png and
// tray_transcribing.png), so all three states are the same brightness.
export const WHITE = [240, 240, 240];
export const TEAL = [0, 140, 158];

// The tile: inset like the recording dot, with the app icon's corner ratio.
export const TILE = { x: 4, size: 56, corner: 0.225 };
// The cut-out bars, centred: logo order (short, mid, tall, mid, short).
export const BARS = { width: 6, gap: 3, heights: [10, 20, 30, 20, 10] };

function symbolicIdleSvg() {
  const { width: w, gap, heights } = BARS;
  const span = heights.length * w + (heights.length - 1) * gap;
  const x0 = SIZE / 2 - span / 2;
  const cuts = heights
    .map(
      (h, i) =>
        `<rect x="${x0 + i * (w + gap)}" y="${SIZE / 2 - h / 2}" width="${w}" height="${h}" rx="${w / 2}" fill="black"/>`,
    )
    .join("");
  const [r, g, b] = WHITE;
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" width="${SIZE}" height="${SIZE}" viewBox="0 0 ${SIZE} ${SIZE}">` +
    `<mask id="cut"><rect width="${SIZE}" height="${SIZE}" fill="white"/>${cuts}</mask>` +
    `<rect x="${TILE.x}" y="${TILE.x}" width="${TILE.size}" height="${TILE.size}" rx="${TILE.size * TILE.corner}" fill="rgb(${r},${g},${b})" mask="url(#cut)"/>` +
    `</svg>`
  );
}

/** One flat colour with the rasterised shape as alpha. */
async function flat(alpha, [r, g, b]) {
  const px = Buffer.alloc(SIZE * SIZE * 4);
  for (let i = 0; i < SIZE * SIZE; i++) {
    px[i * 4] = r;
    px[i * 4 + 1] = g;
    px[i * 4 + 2] = b;
    px[i * 4 + 3] = alpha[i];
  }
  return sharp(px, { raw: { width: SIZE, height: SIZE, channels: 4 } })
    .png({ compressionLevel: 9 })
    .toBuffer();
}

async function alphaOf(input) {
  const { data, info } = await sharp(input)
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  if (info.width !== SIZE || info.height !== SIZE)
    throw new Error(
      `expected ${SIZE}x${SIZE}, got ${info.width}x${info.height}`,
    );
  const alpha = Buffer.alloc(SIZE * SIZE);
  for (let i = 0; i < SIZE * SIZE; i++) alpha[i] = data[i * 4 + 3];
  return alpha;
}

/** src-tauri/resources/tray_idle_symbolic.png: the white tile glyph. */
export async function renderSymbolicIdle() {
  // Rasterise at 4x and reduce, so the cut-outs' edges are antialiased.
  const big = await sharp(Buffer.from(symbolicIdleSvg()), { density: 72 * 4 })
    .resize(SIZE, SIZE, { kernel: "lanczos3" })
    .png()
    .toBuffer();
  return flat(await alphaOf(big), WHITE);
}

/** recording.png / transcribing.png: the white state glyph's shape in teal. */
export async function renderColorState(whiteGlyph) {
  return flat(await alphaOf(whiteGlyph), TEAL);
}

export const OUTPUTS = [
  ["tray_idle_symbolic.png", () => renderSymbolicIdle()],
  ["recording.png", () => renderColorState(RES("tray_recording.png"))],
  ["transcribing.png", () => renderColorState(RES("tray_transcribing.png"))],
];

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  for (const [name, render] of OUTPUTS) {
    fs.writeFileSync(RES(name), await render());
    console.log(
      `src-tauri/resources/${name.padEnd(24)} ${fs.statSync(RES(name)).size}b`,
    );
  }
}
