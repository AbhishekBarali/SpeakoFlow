// Build the FINAL SpeakoFlow logo: the white mark composited inside the
// brand teal rounded box (app-icon style), plus a horizontal wordmark lockup
// (teal box + two-tone "SpeakoFlow"). Reuses the mark paths from
// "Only Logo.svg" and the same wordmark font/layout as build-logos.mjs.
//
// Outputs -> logo/final/  and the two named deliverables in logo/:
//   - "Final logo.png"            (1024 teal-boxed icon)
//   - "Final logo with text.png"  (teal box + "SpeakoFlow" lockup)

import fs from "node:fs";
import path from "node:path";
import sharp from "sharp";
import pngToIco from "png-to-ico";
import getBounds from "svg-path-bounds";
import opentype from "opentype.js";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------
const TEAL = "#14b8a6"; // brand teal box fill (vibrant, matches the app icon)
const WHITE = "#ffffff"; // the mark, on top of the teal box
const INK = "#201a16"; // "Speako" wordmark color (warm near-black brand ink)
const CORNER = 0.225; // rounded-box corner radius as a fraction of the size
const MARK_RATIO = 0.6; // mark's max dimension as a fraction of the box
const OUT = "final";
const PNG_DIR = path.join(OUT, "png");
const ICON_SIZES = [16, 32, 48, 64, 128, 180, 192, 256, 512, 1024];
const transparent = { r: 0, g: 0, b: 0, alpha: 0 };

fs.rmSync(OUT, { recursive: true, force: true });
for (const d of [OUT, PNG_DIR]) fs.mkdirSync(d, { recursive: true });

// ---------------------------------------------------------------------------
// 1. Extract the raw mark paths + tight bounds from the source SVG
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

// ---------------------------------------------------------------------------
// 2. Teal-boxed icon SVG (white mark centered on a rounded teal square)
// ---------------------------------------------------------------------------
// Scale the mark so its larger dimension fills MARK_RATIO of the box, centered.
function boxedIconSvg(px) {
  const B = px;
  const markScale = (B * MARK_RATIO) / Math.max(artW, artH);
  const scaledW = artW * markScale;
  const scaledH = artH * markScale;
  const offX = (B - scaledW) / 2;
  const offY = (B - scaledH) / 2;
  const rx = (B * CORNER).toFixed(3);
  const marks = paths
    .map((d) => `    <path fill="${WHITE}" d="${d}"/>`)
    .join("\n");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${B}" height="${B}" viewBox="0 0 ${B} ${B}">
  <rect x="0" y="0" width="${B}" height="${B}" rx="${rx}" ry="${rx}" fill="${TEAL}"/>
  <g transform="translate(${offX.toFixed(3)} ${offY.toFixed(3)}) scale(${markScale.toFixed(6)}) translate(${(-minX).toFixed(3)} ${(-minY).toFixed(3)})">
${marks}
  </g>
</svg>
`;
}

// A 1024 master SVG (viewBox only, resolution-independent).
fs.writeFileSync(path.join(OUT, "icon-square.svg"), boxedIconSvg(1024));

for (const size of ICON_SIZES) {
  await sharp(Buffer.from(boxedIconSvg(size)), { density: 384 })
    .resize(size, size, { fit: "contain", background: transparent })
    .png()
    .toFile(path.join(PNG_DIR, `icon-${size}.png`));
}
console.log(`Wrote boxed icon PNGs: ${ICON_SIZES.join(", ")}`);

// icon.ico packed with the common Windows sizes.
const icoBufs = await Promise.all(
  [16, 32, 48, 64, 128, 256].map((s) =>
    sharp(Buffer.from(boxedIconSvg(s)), { density: 384 })
      .resize(s, s, { fit: "contain", background: transparent })
      .png()
      .toBuffer(),
  ),
);
fs.writeFileSync(path.join(OUT, "speakoflow.ico"), await pngToIco(icoBufs));
console.log("Wrote speakoflow.ico (16/32/48/64/128/256).");

// Named deliverable: the final boxed logo at 1024.
fs.copyFileSync(
  path.join(PNG_DIR, "icon-1024.png"),
  path.join("..", "logo", "Final logo.png"),
);

// ---------------------------------------------------------------------------
// 3. Wordmark lockup: teal box + two-tone "SpeakoFlow"
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

const FONT_SIZE = 64;
const LETTER_SPACING = -2;
const ICON_H = 88; // box height in the lockup
const GAP = 20;
const scale = FONT_SIZE / font.unitsPerEm;

// Lay out "SpeakoFlow" continuously, but split the fill: Speako=ink, Flow=teal.
const segments = [
  { text: "Speako", fill: INK },
  { text: "Flow", fill: TEAL },
];
const allGlyphs = [...segments.flatMap((s) => [...s.text])].map((ch) =>
  font.charToGlyph(ch),
);
let penX = 0;
let gi = 0;
const segPaths = [];
let fullMinX = Infinity,
  fullMaxX = -Infinity,
  fullMinY = Infinity,
  fullMaxY = -Infinity;
for (const seg of segments) {
  const p = new opentype.Path();
  for (const ch of seg.text) {
    const g = allGlyphs[gi];
    if (gi > 0) penX += font.getKerningValue(allGlyphs[gi - 1], g) * scale;
    p.extend(g.getPath(penX, 0, FONT_SIZE));
    penX += g.advanceWidth * scale + LETTER_SPACING;
    gi++;
  }
  const bb = p.getBoundingBox();
  fullMinX = Math.min(fullMinX, bb.x1);
  fullMaxX = Math.max(fullMaxX, bb.x2);
  fullMinY = Math.min(fullMinY, bb.y1);
  fullMaxY = Math.max(fullMaxY, bb.y2);
  segPaths.push({ d: pathToD(p, 3), fill: seg.fill });
}

const wb = { x1: fullMinX, x2: fullMaxX, y1: fullMinY, y2: fullMaxY };
const boxCenterY = ICON_H / 2;
const wordMidY = (wb.y1 + wb.y2) / 2;
const baselineY = boxCenterY - wordMidY;
const textX = ICON_H + GAP - wb.x1;

const contentMaxX = Math.max(ICON_H, textX + wb.x2);
const contentMinY = Math.min(0, baselineY + wb.y1);
const contentMaxY = Math.max(ICON_H, baselineY + wb.y2);
const M = 14;
const lockMinX = -M;
const lockMinY = contentMinY - M;
const lockW = contentMaxX + 2 * M;
const lockH = contentMaxY - contentMinY + 2 * M;

// Teal box + white mark for the lockup icon.
const boxMarkScale = (ICON_H * MARK_RATIO) / Math.max(artW, artH);
const bmScaledW = artW * boxMarkScale;
const bmScaledH = artH * boxMarkScale;
const bmOffX = (ICON_H - bmScaledW) / 2;
const bmOffY = (ICON_H - bmScaledH) / 2;
const boxRx = (ICON_H * CORNER).toFixed(3);
const lockupMarks = paths
  .map((d) => `      <path fill="${WHITE}" d="${d}"/>`)
  .join("\n");

const lockupSvg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${lockMinX.toFixed(3)} ${lockMinY.toFixed(3)} ${lockW.toFixed(3)} ${lockH.toFixed(3)}">
  <rect x="0" y="0" width="${ICON_H}" height="${ICON_H}" rx="${boxRx}" ry="${boxRx}" fill="${TEAL}"/>
  <g transform="translate(${bmOffX.toFixed(3)} ${bmOffY.toFixed(3)}) scale(${boxMarkScale.toFixed(6)}) translate(${(-minX).toFixed(3)} ${(-minY).toFixed(3)})">
${lockupMarks}
  </g>
  <g transform="translate(${textX.toFixed(3)} ${baselineY.toFixed(3)})">
${segPaths.map((s) => `    <path fill="${s.fill}" d="${s.d}"/>`).join("\n")}
  </g>
</svg>
`;
fs.writeFileSync(path.join(OUT, "lockup.svg"), lockupSvg);

const aspect = lockW / lockH;
for (const h of [64, 128, 256, 512]) {
  const w = Math.round(h * aspect);
  await sharp(Buffer.from(lockupSvg), { density: 384 })
    .resize(w, h, { fit: "contain", background: transparent })
    .png()
    .toFile(path.join(PNG_DIR, `lockup-h${h}.png`));
}
console.log("Wrote lockup PNGs (h64/h128/h256/h512).");

// Named deliverable: the final lockup with text (512 tall for crispness).
fs.copyFileSync(
  path.join(PNG_DIR, "lockup-h512.png"),
  path.join("..", "logo", "Final logo with text.png"),
);

console.log("\nDONE. Assets in logo/final/ ; deliverables in logo/.");
