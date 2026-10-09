// Build the macOS menu-bar idle icon: src-tauri/resources/tray_idle_template.png
//
// WHY THIS IS NOT THE APP TILE
// macOS draws every SpeakoFlow tray icon as a *template image*
// (`icon_as_template(true)` in lib.rs, `set_icon_as_template(true)` in
// tray.rs). A template image is drawn from its alpha channel only: the RGB is
// thrown away and every opaque pixel becomes the menu bar's text colour. The
// colour tile that apply-icon-to-app.mjs writes to tray_idle.png has solid alpha
// from edge to edge, and its waveform differs from the teal only in colour, so
// the menu bar showed a plain white (or black) rounded square with no logo in
// it. Windows and Linux draw the RGB, which is why only Mac users saw this.
//
// This glyph is the same mark drawn in alpha: the rounded tile with the five
// waveform bars (short, medium, tall, medium, short) cut out of it, so the bars
// show the menu bar through them. It is drawn for the size macOS uses: the tray
// crate scales the image to 18pt tall, so 36px is exactly @2x and every edge
// below lands on a whole pixel. The tile is 16pt with a 1pt margin, like other
// menu-bar extras, and its corner radius is the app icon's 22.5%.
//
// The recording and transcribing glyphs are already alpha-shaped and are left
// alone. Windows and Linux keep the colour tile.
//
// Run from the Logo folder: node build-tray-template.mjs

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const SIZE = 36; // 18pt @2x
const TILE = { inset: 2, size: 32, radius: 32 * 0.225 };
const BAR_WIDTH = 2;
const BAR_GAP = 2;
const BAR_HEIGHTS = [6, 10, 16, 10, 6];

// Resolved from this file, so the verifier can import the renderer from the
// repo root while this script is run from the Logo folder.
const OUT = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "src-tauri",
  "resources",
  "tray_idle_template.png",
);

function bars() {
  const total =
    BAR_HEIGHTS.length * BAR_WIDTH + (BAR_HEIGHTS.length - 1) * BAR_GAP;
  let x = (SIZE - total) / 2;
  return BAR_HEIGHTS.map((h) => {
    const bar = { x, y: (SIZE - h) / 2, w: BAR_WIDTH, h };
    x += BAR_WIDTH + BAR_GAP;
    return bar;
  });
}

export function trayTemplateSvg() {
  const cutouts = bars()
    .map(
      ({ x, y, w, h }) =>
        `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${w / 2}" fill="#000"/>`,
    )
    .join("");
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" width="${SIZE}" height="${SIZE}" viewBox="0 0 ${SIZE} ${SIZE}">` +
    `<defs><mask id="tile">` +
    `<rect x="${TILE.inset}" y="${TILE.inset}" width="${TILE.size}" height="${TILE.size}" rx="${TILE.radius}" fill="#fff"/>` +
    cutouts +
    `</mask></defs>` +
    // Black on transparent, the convention for template images. Only the alpha
    // matters to macOS.
    `<rect width="${SIZE}" height="${SIZE}" fill="#000" mask="url(#tile)"/>` +
    `</svg>`
  );
}

export function renderTrayTemplate() {
  return sharp(Buffer.from(trayTemplateSvg()))
    .png({ compressionLevel: 9 })
    .toBuffer();
}

if (path.resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  fs.writeFileSync(OUT, await renderTrayTemplate());
  console.log(
    `${path.relative(process.cwd(), OUT)} ${SIZE}x${SIZE} ${fs.statSync(OUT).size}b`,
  );
}
