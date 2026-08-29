// Emit the small-tier reference assets (see small-icon.mjs for the design notes).
// These are the canonical small-size renders; apply-icon-to-app.mjs imports the
// same generator so the app assets and this reference set cannot drift.
//
// Run from the Logo folder: node build-small-icon.mjs

import fs from "node:fs";
import path from "node:path";
import sharp from "sharp";
import pngToIco from "png-to-ico";
import { smallIconSvg } from "./small-icon.mjs";

const OUT = path.join("final-v2", "small");
const PNG_DIR = path.join(OUT, "png");
const SIZES = [16, 20, 24, 32, 40, 48, 64];
const ICO_SIZES = [16, 20, 24, 32, 48, 64];
const REFERENCE = 1024; // for design review only; nothing consumes it at this size

fs.rmSync(OUT, { recursive: true, force: true });
fs.mkdirSync(PNG_DIR, { recursive: true });

/** Render the vector at its own target size -- never by scaling another raster. */
export const renderSmall = (size) =>
  sharp(Buffer.from(smallIconSvg(size)), { density: 384 })
    .resize(size, size)
    .png({ compressionLevel: 9 })
    .toBuffer();

const rendered = new Map();
for (const size of SIZES) {
  const buf = await renderSmall(size);
  rendered.set(size, buf);
  fs.writeFileSync(path.join(PNG_DIR, `icon-${size}.png`), buf);
}
fs.writeFileSync(path.join(OUT, "icon-small.svg"), smallIconSvg(REFERENCE));
fs.writeFileSync(
  path.join(OUT, `small-reference-${REFERENCE}.png`),
  await renderSmall(REFERENCE),
);
console.log(`wrote ${OUT}/png/icon-{${SIZES.join(",")}}.png + icon-small.svg`);

// A multi-size ICO of the small tier on its own, so a shell handed this file can
// pick an exact entry instead of scaling one bitmap.
fs.writeFileSync(
  path.join(OUT, "icon-small.ico"),
  await pngToIco(ICO_SIZES.map((s) => rendered.get(s))),
);
console.log(`wrote ${OUT}/icon-small.ico (${ICO_SIZES.join("/")})`);

// Report the snapped bar metrics per size: this is the number that matters, since
// a sub-pixel bar is what made the old small renders look smeared.
for (const size of [16, 20, 24, 32, 48, 64]) {
  const pills = [
    ...smallIconSvg(size).matchAll(
      /<rect x="(\d+)" y="(\d+)" width="(\d+)" height="(\d+)"[^>]*fill="#ffffff"/g,
    ),
  ].map((m) => ({ x: +m[1], y: +m[2], w: +m[3], h: +m[4] }));
  const gap = pills.length > 1 ? pills[1].x - (pills[0].x + pills[0].w) : "-";
  const span = pills.length
    ? pills[pills.length - 1].x + pills[pills.length - 1].w - pills[0].x
    : 0;
  console.log(
    `  ${String(size).padStart(3)}px: ${pills.length} bars, ${pills[0]?.w}px wide,` +
      ` ${gap}px gaps, span ${span}px (${((span / size) * 100).toFixed(0)}% of tile),` +
      ` tallest ${Math.max(...pills.map((p) => p.h))}px`,
  );
}
