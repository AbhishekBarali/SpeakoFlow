import fs from "node:fs";
import path from "node:path";
import sharp from "sharp";
import pngToIco from "png-to-ico";
import getBounds from "svg-path-bounds";
import opentype from "opentype.js";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------
const BRAND = "#201a16"; // warm near-black brand color (from the finished lockup)
const WHITE = "#ffffff";
const OUT = "SpeakoFlow-logo";
const SVG_DIR = path.join(OUT, "svg");
const PNG_DIR = path.join(OUT, "png");
const ICON_SIZES = [16, 32, 48, 64, 128, 180, 192, 256, 512, 1024];

// Two color options for every raster asset.
const VARIANTS = [
  { name: "black", fill: BRAND },
  { name: "white", fill: WHITE },
];

for (const d of [OUT, SVG_DIR, PNG_DIR]) fs.mkdirSync(d, { recursive: true });
// Fresh PNG tree (remove any previous flat files) then per-variant subfolders.
fs.rmSync(PNG_DIR, { recursive: true, force: true });
for (const v of VARIANTS)
  fs.mkdirSync(path.join(PNG_DIR, v.name), { recursive: true });

// ---------------------------------------------------------------------------
// 1. Extract the raw icon paths from the original SVG + compute tight bounds
// ---------------------------------------------------------------------------
const original = fs.readFileSync("Only Logo.svg", "utf8");
const paths = [...original.matchAll(/\sd="([^"]+)"/g)].map((m) =>
  m[1].replace(/\s+/g, " ").trim(),
);

let minX = Infinity,
  minY = Infinity,
  maxX = -Infinity,
  maxY = -Infinity;
for (const d of paths) {
  const [x0, y0, x1, y1] = getBounds(d);
  minX = Math.min(minX, x0);
  minY = Math.min(minY, y0);
  maxX = Math.max(maxX, x1);
  maxY = Math.max(maxY, y1);
}
const artW = maxX - minX;
const artH = maxY - minY;
const cx = (minX + maxX) / 2;
const cy = (minY + maxY) / 2;
console.log(
  `Artwork bbox: ${artW.toFixed(2)} x ${artH.toFixed(2)} @ center (${cx.toFixed(2)}, ${cy.toFixed(2)})`,
);

const pathsSvg = (fill) =>
  paths.map((d) => `  <path fill="${fill}" d="${d}"/>`).join("\n");

// ---------------------------------------------------------------------------
// 2. Master SVGs
// ---------------------------------------------------------------------------
// Square master: centered artwork with a balanced safe margin.
const SQUARE = Math.max(artW, artH) * 1.21; // ~9% margin on the wide axis
const sqMinX = cx - SQUARE / 2;
const sqMinY = cy - SQUARE / 2;
const squareViewBox = `${sqMinX.toFixed(3)} ${sqMinY.toFixed(3)} ${SQUARE.toFixed(3)} ${SQUARE.toFixed(3)}`;

const squareSvg = (fill, px) => `<svg xmlns="http://www.w3.org/2000/svg"${
  px ? ` width="${px}" height="${px}"` : ""
} viewBox="${squareViewBox}">
${pathsSvg(fill)}
</svg>
`;

// Tight master: exact artwork bounds, no padding (for custom lockups / embedding).
const tightViewBox = `${minX.toFixed(3)} ${minY.toFixed(3)} ${artW.toFixed(3)} ${artH.toFixed(3)}`;
const tightSvg = (
  fill,
) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${tightViewBox}">
${pathsSvg(fill)}
</svg>
`;

fs.writeFileSync(path.join(SVG_DIR, "icon-square.svg"), squareSvg(BRAND));
fs.writeFileSync(path.join(SVG_DIR, "icon-square-white.svg"), squareSvg(WHITE));
fs.writeFileSync(path.join(SVG_DIR, "icon-tight.svg"), tightSvg(BRAND));
fs.writeFileSync(path.join(SVG_DIR, "icon-tight-white.svg"), tightSvg(WHITE));
console.log("Wrote square + tight master SVGs (black + white).");

// ---------------------------------------------------------------------------
// 3. PNG icon set (transparent background) - black + white
// ---------------------------------------------------------------------------
const transparent = { r: 0, g: 0, b: 0, alpha: 0 };
for (const v of VARIANTS) {
  for (const size of ICON_SIZES) {
    const buf = Buffer.from(squareSvg(v.fill, size));
    await sharp(buf, { density: 384 })
      .resize(size, size, { fit: "contain", background: transparent })
      .png()
      .toFile(path.join(PNG_DIR, v.name, `icon-${size}.png`));
  }
}
console.log(
  `Wrote ${ICON_SIZES.length} icons x2 variants: ${ICON_SIZES.join(", ")}`,
);

// ---------------------------------------------------------------------------
// 4. favicon.ico (16 / 32 / 48 packed) - black + white
// ---------------------------------------------------------------------------
for (const v of VARIANTS) {
  const icoBufs = await Promise.all(
    [16, 32, 48].map((s) =>
      sharp(Buffer.from(squareSvg(v.fill, s)), { density: 384 })
        .resize(s, s, { fit: "contain", background: transparent })
        .png()
        .toBuffer(),
    ),
  );
  const name = v.name === "black" ? "favicon.ico" : "favicon-white.ico";
  fs.writeFileSync(path.join(OUT, name), await pngToIco(icoBufs));
}
console.log("Wrote favicon.ico + favicon-white.ico (16/32/48).");

// ---------------------------------------------------------------------------
// 5. Wordmark lockup ("SpeakoFlow") -> vector paths, aligned with the icon
// ---------------------------------------------------------------------------
const fontBuf = fs.readFileSync("fonts/PlusJakartaSans-800.ttf");
const font = opentype.parse(
  fontBuf.buffer.slice(
    fontBuf.byteOffset,
    fontBuf.byteOffset + fontBuf.byteLength,
  ),
);

// opentype.js's built-in toPathData() can emit "NaN" control points for some
// glyphs at certain positions (its serializer bug), which collapses a letter's
// inner counter and renders e.g. "o" as a solid blob. The raw command
// coordinates are correct, so we serialize the path ourselves.
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
  if (d.includes("NaN"))
    throw new Error("wordmark path serialization produced NaN");
  return d;
}
const TEXT = "SpeakoFlow";
const FONT_SIZE = 64;
const LETTER_SPACING = -2; // matches the reference CSS letter-spacing: -2px
const ICON_H = 88; // reference icon height
const GAP = 18; // reference gap between icon and text

// Build a single path for the wordmark with manual kerning + letter-spacing.
const scale = FONT_SIZE / font.unitsPerEm;
const glyphs = [...TEXT].map((ch) => font.charToGlyph(ch));
const wordPath = new opentype.Path();
let penX = 0;
for (let i = 0; i < glyphs.length; i++) {
  const g = glyphs[i];
  if (i > 0) penX += font.getKerningValue(glyphs[i - 1], g) * scale;
  const gp = g.getPath(penX, 0, FONT_SIZE); // baseline at y=0
  wordPath.extend(gp);
  penX += g.advanceWidth * scale + LETTER_SPACING;
}
const wb = wordPath.getBoundingBox(); // {x1,y1,x2,y2}, y up-is-negative (baseline 0)

// Icon scaled to reference height.
const iconScale = ICON_H / artH;
const iconWpx = artW * iconScale;

// Vertically center the icon's optical middle with the wordmark's optical middle.
const iconTop = 0;
const iconCenterY = iconTop + ICON_H / 2;
const wordMidY = (wb.y1 + wb.y2) / 2;
const baselineY = iconCenterY - wordMidY;

// Horizontal: wordmark starts right after icon + gap, flush to its left edge.
const textX = iconWpx + GAP - wb.x1;

// Union bounding box of icon + wordmark, then a small uniform margin.
const contentMinX = 0;
const contentMaxX = Math.max(iconWpx, textX + wb.x2);
const contentMinY = Math.min(iconTop, baselineY + wb.y1);
const contentMaxY = Math.max(iconTop + ICON_H, baselineY + wb.y2);
const M = 14;
const lockMinX = contentMinX - M;
const lockMinY = contentMinY - M;
const lockW = contentMaxX - contentMinX + 2 * M;
const lockH = contentMaxY - contentMinY + 2 * M;

const iconTransform = `translate(${iconTop === 0 ? 0 : 0} ${iconTop}) scale(${iconScale}) translate(${(-minX).toFixed(3)} ${(-minY).toFixed(3)})`;

const wordmarkSvg = (
  iconFill,
  textFill,
) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${lockMinX.toFixed(3)} ${lockMinY.toFixed(3)} ${lockW.toFixed(3)} ${lockH.toFixed(3)}">
  <g transform="scale(${iconScale}) translate(${(-minX).toFixed(3)} ${(-minY).toFixed(3)})">
${paths.map((d) => `    <path fill="${iconFill}" d="${d}"/>`).join("\n")}
  </g>
  <g transform="translate(${textX.toFixed(3)} ${baselineY.toFixed(3)})">
    <path fill="${textFill}" d="${pathToD(wordPath, 3)}"/>
  </g>
</svg>
`;

fs.writeFileSync(path.join(SVG_DIR, "wordmark.svg"), wordmarkSvg(BRAND, BRAND));
fs.writeFileSync(
  path.join(SVG_DIR, "wordmark-white.svg"),
  wordmarkSvg(WHITE, WHITE),
);
console.log("Wrote wordmark SVGs.");

// Wordmark PNGs at a few useful heights (transparent) - black + white.
const aspect = lockW / lockH;
for (const v of VARIANTS) {
  for (const h of [64, 128, 256]) {
    const w = Math.round(h * aspect);
    await sharp(Buffer.from(wordmarkSvg(v.fill, v.fill)), { density: 384 })
      .resize(w, h, { fit: "contain", background: transparent })
      .png()
      .toFile(path.join(PNG_DIR, v.name, `wordmark-h${h}.png`));
  }
}
console.log("Wrote wordmark PNGs x2 variants (h64/h128/h256).");

// ---------------------------------------------------------------------------
// 6. Text-only wordmark ("SpeakoFlow" letters, no icon) - black + white
// ---------------------------------------------------------------------------
// wordPath baseline is at y=0; wb.y1 is the cap top (negative), wb.y2 the
// descender bottom (positive). Crop tightly to the letters with a small margin.
const TM = 6;
const txtVbX = wb.x1 - TM;
const txtVbY = wb.y1 - TM;
const txtVbW = wb.x2 - wb.x1 + 2 * TM;
const txtVbH = wb.y2 - wb.y1 + 2 * TM;
const textOnlySvg = (fill) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${txtVbX.toFixed(3)} ${txtVbY.toFixed(
    3,
  )} ${txtVbW.toFixed(3)} ${txtVbH.toFixed(3)}">
  <path fill="${fill}" d="${pathToD(wordPath, 3)}"/>
</svg>
`;

fs.writeFileSync(path.join(SVG_DIR, "wordmark-text.svg"), textOnlySvg(BRAND));
fs.writeFileSync(
  path.join(SVG_DIR, "wordmark-text-white.svg"),
  textOnlySvg(WHITE),
);

const textAspect = txtVbW / txtVbH;
for (const v of VARIANTS) {
  for (const h of [64, 128, 256]) {
    const w = Math.round(h * textAspect);
    await sharp(Buffer.from(textOnlySvg(v.fill)), { density: 384 })
      .resize(w, h, { fit: "contain", background: transparent })
      .png()
      .toFile(path.join(PNG_DIR, v.name, `wordmark-text-h${h}.png`));
  }
}
console.log("Wrote text-only wordmark SVG + PNG x2 variants.");

console.log("\nDONE. Output in", OUT + "/");
