// Build the brand lockup ("SpeakoFlow" wordmark beside the app tile) for the
// current icon, plus the named deliverables and the docs-site logo files.
//
// Geometry (tile 88px, gap 20, 14px margin, Plus Jakarta Sans 800 at 64px with
// -2 tracking) is copied from build-final-logo.mjs on purpose, so the lockup
// keeps the same proportions as before -- only the tile artwork changes. The new
// tile is raster, so it is embedded as a base64 <image> rather than drawn as
// vector paths; the wordmark stays real vector text.
//
// Run from the Logo folder: node build-new-lockup.mjs

import fs from "node:fs";
import path from "node:path";
import sharp from "sharp";
import opentype from "opentype.js";

const MASTER = path.join("final-v2", "icon-master-2048.png");
const OUT = "final-v2";
const PNG_DIR = path.join(OUT, "png");
const REPO = "..";

const TEAL = "#14b8a6"; // "Flow" half of the wordmark
const INK = "#201a16"; // "Speako" on light backgrounds
const PAPER = "#ffffff"; // "Speako" on dark backgrounds
const ICON_H = 88; // tile height inside the lockup
const GAP = 20;
const M = 14; // margin around the lockup content
const FONT_SIZE = 64;
const LETTER_SPACING = -2;
const TILE_RASTER = 256; // embedded tile resolution (~4x the drawn size)
const transparent = { r: 0, g: 0, b: 0, alpha: 0 };

// ---------------------------------------------------------------------------
// Wordmark geometry
// ---------------------------------------------------------------------------
const fontBuf = fs.readFileSync("fonts/PlusJakartaSans-800.ttf");
const font = opentype.parse(
  fontBuf.buffer.slice(
    fontBuf.byteOffset,
    fontBuf.byteOffset + fontBuf.byteLength,
  ),
);

// opentype.js's toPathData() can emit NaN control points; serialize manually.
function pathToD(pathObj, dp = 3) {
  const r = (n) => {
    const v = Number(n.toFixed(dp));
    return Object.is(v, -0) ? 0 : v;
  };
  let d = "";
  for (const c of pathObj.commands) {
    if (c.type === "M") d += `M${r(c.x)} ${r(c.y)}`;
    else if (c.type === "L") d += `L${r(c.x)} ${r(c.y)}`;
    else if (c.type === "C")
      d += `C${r(c.x1)} ${r(c.y1)} ${r(c.x2)} ${r(c.y2)} ${r(c.x)} ${r(c.y)}`;
    else if (c.type === "Q") d += `Q${r(c.x1)} ${r(c.y1)} ${r(c.x)} ${r(c.y)}`;
    else if (c.type === "Z") d += "Z";
  }
  if (d.includes("NaN")) throw new Error("wordmark serialization produced NaN");
  return d;
}

const scale = FONT_SIZE / font.unitsPerEm;
// Laid out as one continuous word, then split by fill: Speako = ink, Flow = teal.
const segments = ["Speako", "Flow"];
const allGlyphs = [...segments.join("")].map((ch) => font.charToGlyph(ch));
let penX = 0;
let gi = 0;
const segPaths = [];
let minX = Infinity,
  maxX = -Infinity,
  minY = Infinity,
  maxY = -Infinity;
for (const text of segments) {
  const p = new opentype.Path();
  for (const _ of text) {
    const g = allGlyphs[gi];
    if (gi > 0) penX += font.getKerningValue(allGlyphs[gi - 1], g) * scale;
    p.extend(g.getPath(penX, 0, FONT_SIZE));
    penX += g.advanceWidth * scale + LETTER_SPACING;
    gi++;
  }
  const bb = p.getBoundingBox();
  minX = Math.min(minX, bb.x1);
  maxX = Math.max(maxX, bb.x2);
  minY = Math.min(minY, bb.y1);
  maxY = Math.max(maxY, bb.y2);
  segPaths.push(pathToD(p, 3));
}

const baselineY = ICON_H / 2 - (minY + maxY) / 2;
const textX = ICON_H + GAP - minX;
const contentMaxX = Math.max(ICON_H, textX + maxX);
const contentMinY = Math.min(0, baselineY + minY);
const contentMaxY = Math.max(ICON_H, baselineY + maxY);
const viewBox = [
  -M,
  contentMinY - M,
  contentMaxX + 2 * M,
  contentMaxY - contentMinY + 2 * M,
];

// ---------------------------------------------------------------------------
// Lockup SVGs
// ---------------------------------------------------------------------------
const tileB64 = (
  await sharp(MASTER)
    .resize(TILE_RASTER, TILE_RASTER, { kernel: "lanczos3" })
    .png({ compressionLevel: 9 })
    .toBuffer()
).toString("base64");

function lockupSvg(speakoFill) {
  return `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="${viewBox.map((n) => n.toFixed(3)).join(" ")}">
  <title>SpeakoFlow</title>
  <image x="0" y="0" width="${ICON_H}" height="${ICON_H}" xlink:href="data:image/png;base64,${tileB64}"/>
  <g transform="translate(${textX.toFixed(3)} ${baselineY.toFixed(3)})">
    <path fill="${speakoFill}" d="${segPaths[0]}"/>
    <path fill="${TEAL}" d="${segPaths[1]}"/>
  </g>
</svg>
`;
}

const lockupLight = lockupSvg(INK);
const lockupDark = lockupSvg(PAPER);
fs.writeFileSync(path.join(OUT, "lockup.svg"), lockupLight);
fs.writeFileSync(path.join(OUT, "lockup-dark.svg"), lockupDark);
console.log("wrote final-v2/lockup.svg + lockup-dark.svg");

// Raster lockups, for slots that cannot take an SVG -- and for the README,
// where GitHub's SVG sanitizer makes an embedded-raster SVG a gamble.
const aspect = viewBox[2] / viewBox[3];
for (const [variant, svg] of [
  ["", lockupLight],
  ["-dark", lockupDark],
]) {
  for (const h of [64, 128, 256, 512]) {
    await sharp(Buffer.from(svg), { density: 384 })
      .resize(Math.round(h * aspect), h, {
        fit: "contain",
        background: transparent,
      })
      .png({ compressionLevel: 9 })
      .toFile(path.join(PNG_DIR, `lockup${variant}-h${h}.png`));
  }
}
console.log("wrote lockup + lockup-dark PNGs (h64/h128/h256/h512)");

// ---------------------------------------------------------------------------
// Named deliverables + docs site
// ---------------------------------------------------------------------------
fs.copyFileSync(path.join(PNG_DIR, "icon-1024.png"), "Final logo.png");
fs.copyFileSync(
  path.join(PNG_DIR, "lockup-h512.png"),
  "Final logo with text.png",
);
console.log('wrote "Final logo.png" + "Final logo with text.png"');

const docsLogo = path.join(REPO, "docs-site", "logo");
if (fs.existsSync(docsLogo)) {
  fs.writeFileSync(path.join(docsLogo, "lockup.svg"), lockupLight);
  fs.writeFileSync(path.join(docsLogo, "lockup-dark.svg"), lockupDark);
  fs.copyFileSync(
    path.join(OUT, "icon-square.svg"),
    path.join(docsLogo, "icon.svg"),
  );
  console.log("wrote docs-site/logo/{lockup,lockup-dark,icon}.svg");
}

console.log("\nDONE.");
