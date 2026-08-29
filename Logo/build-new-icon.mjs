// Normalize the new SpeakoFlow app-icon artwork and emit the full size set.
//
// The hand-off PNG ("Speakoflow new Logo.png") is a 1254x1254 canvas holding an
// 826x847 rounded-square tile that is off-centre (padding 179/209/249/198) and
// 2.5% taller than wide. Dropped into any icon slot it therefore renders small,
// visually off-centre, and slightly stretched -- unlike the previous icon, whose
// teal tile bleeds to all four edges with a 0.225 corner radius.
//
// This script rebuilds it to that same geometry:
//   1. crop to the tight alpha bounding box (kills the dead padding)
//   2. resize to a square master (removes the out-of-square distortion)
//   3. edge-extend every row outward so the tile is fully opaque corner to
//      corner -- the mask, not the source's anti-aliased edge, then defines the
//      silhouette, so no semi-transparent fringe or flat sliver survives
//   4. mask with a clean rounded rect at CORNER, matching the old brand tile
//   5. downsample to each delivered size from the 2048 master
//
// Outputs -> Logo/final-v2/   (nothing existing is overwritten)
//
// Run:  cd Logo && node build-new-icon.mjs

import fs from "node:fs";
import path from "node:path";
import sharp from "sharp";
import pngToIco from "png-to-ico";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------
const SRC = "Speakoflow new Logo.png";
const OUT = "final-v2";
const PNG_DIR = path.join(OUT, "png");

const CORNER = 0.225; // corner radius / size -- same tile silhouette as Logo/final
const MASTER = 2048; // supersample resolution everything is derived from
const PAD_RATIO = 0.8; // tile size inside the padded variant (macOS / maskable)
const ALPHA_SOLID = 200; // alpha at/above this counts as real artwork, not AA
const ALPHA_ANY = 24; // alpha at/above this counts for the crop bounding box

const SIZES = [16, 24, 32, 48, 64, 96, 128, 144, 180, 192, 256, 384, 512, 1024];
const ICO_SIZES = [16, 32, 48, 64, 128, 256];
const transparent = { r: 0, g: 0, b: 0, alpha: 0 };

// Clean only what this script writes, so hand-kept extras (e.g. preview/) survive.
fs.rmSync(PNG_DIR, { recursive: true, force: true });
fs.mkdirSync(PNG_DIR, { recursive: true });

// ---------------------------------------------------------------------------
// 1. Tight alpha bounding box
// ---------------------------------------------------------------------------
const src = await sharp(SRC)
  .ensureAlpha()
  .raw()
  .toBuffer({ resolveWithObject: true });
const { width: SW, height: SH, channels: SC } = src.info;

let minX = SW,
  minY = SH,
  maxX = -1,
  maxY = -1;
for (let y = 0; y < SH; y++) {
  for (let x = 0; x < SW; x++) {
    if (src.data[(y * SW + x) * SC + 3] >= ALPHA_ANY) {
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (y < minY) minY = y;
      if (y > maxY) maxY = y;
    }
  }
}
if (maxX < 0) throw new Error(`${SRC} has no visible pixels`);
const crop = {
  left: minX,
  top: minY,
  width: maxX - minX + 1,
  height: maxY - minY + 1,
};
console.log(
  `source ${SW}x${SH} -> art ${crop.width}x${crop.height} at (${crop.left},${crop.top})`,
);

// ---------------------------------------------------------------------------
// 2 + 3. Square master, then edge-extend to a fully opaque tile
// ---------------------------------------------------------------------------
const squared = await sharp(SRC)
  .ensureAlpha()
  .extract(crop)
  .resize(MASTER, MASTER, { fit: "fill", kernel: "lanczos3" })
  .raw()
  .toBuffer({ resolveWithObject: true });

const buf = squared.data;
const C = squared.info.channels;
const W = squared.info.width;
const H = squared.info.height;
const idx = (x, y) => (y * W + x) * C;

// Solid horizontal span per row; rows that are pure anti-aliasing (the very top
// and bottom scanlines) inherit the nearest row that has one.
const spans = new Array(H).fill(null);
for (let y = 0; y < H; y++) {
  let first = -1,
    last = -1;
  for (let x = 0; x < W; x++) {
    if (buf[idx(x, y) + 3] >= ALPHA_SOLID) {
      if (first < 0) first = x;
      last = x;
    }
  }
  if (first >= 0) spans[y] = { first, last };
}
const firstSolidRow = spans.findIndex(Boolean);
const lastSolidRow = spans.reduce((acc, s, i) => (s ? i : acc), -1);
if (firstSolidRow < 0) throw new Error("no solid row found in artwork");

for (let y = 0; y < H; y++) {
  // Rows outside the solid range are copied wholesale from the nearest solid
  // row, so the mask still has real colour to cut out of.
  const donorY = spans[y]
    ? y
    : y < firstSolidRow
      ? firstSolidRow
      : lastSolidRow;
  const { first, last } = spans[donorY];
  for (let x = 0; x < W; x++) {
    const clampedX = x < first ? first : x > last ? last : x;
    const s = idx(clampedX, donorY);
    const d = idx(x, y);
    if (s !== d) {
      buf[d] = buf[s];
      buf[d + 1] = buf[s + 1];
      buf[d + 2] = buf[s + 2];
    }
    buf[d + 3] = 255; // opaque everywhere; the mask defines the silhouette
  }
}

// ---------------------------------------------------------------------------
// 4. Rounded-rect mask
// ---------------------------------------------------------------------------
function maskSvg(size) {
  const r = (size * CORNER).toFixed(3);
  return Buffer.from(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}">` +
      `<rect x="0" y="0" width="${size}" height="${size}" rx="${r}" ry="${r}" fill="#fff"/></svg>`,
  );
}

const masterPng = await sharp(buf, {
  raw: { width: W, height: H, channels: C },
})
  .composite([{ input: maskSvg(MASTER), blend: "dest-in" }])
  .png()
  .toBuffer();

// The un-rounded square, for Android adaptive / maskable slots that apply their
// own shape and would otherwise clip our corners twice.
const masterSquarePng = await sharp(buf, {
  raw: { width: W, height: H, channels: C },
})
  .png()
  .toBuffer();

// ---------------------------------------------------------------------------
// 5. Deliverables
// ---------------------------------------------------------------------------
// Below ~64px the waveform bars are only a couple of pixels wide and a plain
// lanczos downsample greys them into the tile. A light unsharp pass restores the
// bar/background separation at tray and favicon sizes without haloing.
const SHARPEN_BELOW = 96;

function resizeTo(input, size) {
  let pipe = sharp(input).resize(size, size, { kernel: "lanczos3" });
  if (size < SHARPEN_BELOW) {
    pipe = pipe.sharpen({
      sigma: 0.6 + 0.6 * (1 - size / SHARPEN_BELOW),
      m1: 0,
      m2: 2.2,
    });
  }
  return pipe.png({ compressionLevel: 9 });
}

for (const size of SIZES) {
  await resizeTo(masterPng, size).toFile(
    path.join(PNG_DIR, `icon-${size}.png`),
  );
}
console.log(`wrote icon-*.png: ${SIZES.join(", ")}`);

await sharp(masterPng).png().toFile(path.join(OUT, "icon-master-2048.png"));
await resizeTo(masterSquarePng, 1024).toFile(
  path.join(PNG_DIR, "icon-square-1024.png"),
);

// Padded variant: macOS app icons and web "maskable" icons expect breathing room
// around the tile rather than a full-bleed one.
for (const size of [512, 1024]) {
  const inner = Math.round(size * PAD_RATIO);
  const tile = await resizeTo(masterPng, inner).toBuffer();
  await sharp({
    create: {
      width: size,
      height: size,
      channels: 4,
      background: transparent,
    },
  })
    .composite([
      {
        input: tile,
        left: Math.round((size - inner) / 2),
        top: Math.round((size - inner) / 2),
      },
    ])
    .png()
    .toFile(path.join(PNG_DIR, `icon-padded-${size}.png`));
}
console.log("wrote icon-square-1024.png and icon-padded-{512,1024}.png");

const icoBufs = await Promise.all(
  ICO_SIZES.map((s) => resizeTo(masterPng, s).toBuffer()),
);
fs.writeFileSync(path.join(OUT, "speakoflow.ico"), await pngToIco(icoBufs));
console.log(`wrote speakoflow.ico (${ICO_SIZES.join("/")})`);

// SVG wrapper around a 512 raster, so the icon can be dropped into SVG-only
// slots (README, web) without re-exporting by hand. The new artwork is raster,
// so this is a container, not a real vector -- 512 keeps the file usable in a
// README instead of megabytes of base64.
const SVG_RASTER = 512;
const b64 = (await resizeTo(masterPng, SVG_RASTER).toBuffer()).toString(
  "base64",
);
fs.writeFileSync(
  path.join(OUT, "icon-square.svg"),
  `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${SVG_RASTER}" height="${SVG_RASTER}" viewBox="0 0 ${SVG_RASTER} ${SVG_RASTER}">
  <image width="${SVG_RASTER}" height="${SVG_RASTER}" xlink:href="data:image/png;base64,${b64}"/>
</svg>
`,
);
console.log(`wrote icon-square.svg (embedded ${SVG_RASTER} raster)`);

console.log(`\nDONE -> ${OUT}/`);
