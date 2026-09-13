// Generate every small-size deliverable from the same imagegen artwork.
// Run from Logo: node build-small-icon.mjs
import fs from "node:fs";
import path from "node:path";
import pngToIco from "png-to-ico";
import { renderSmall, smallIconSvg } from "./small-icon.mjs";

const OUT = path.join("final-v2", "small");
const PNG_DIR = path.join(OUT, "png");
const SIZES = [16, 20, 24, 32, 40, 48, 64];
// Do not delete OUT: it also contains the prompt and review documentation.
fs.mkdirSync(PNG_DIR, { recursive: true });
const rendered = await Promise.all(SIZES.map((size) => renderSmall(size)));
for (const [i, size] of SIZES.entries()) {
  fs.writeFileSync(path.join(PNG_DIR, `icon-${size}.png`), rendered[i]);
}
fs.writeFileSync(path.join(OUT, "icon-small.ico"), await pngToIco(rendered));
fs.writeFileSync(path.join(OUT, "icon-small.svg"), await smallIconSvg(256));
fs.writeFileSync(
  path.join(OUT, "small-reference-1024.png"),
  await renderSmall(1024),
);
console.log(`Updated ${OUT}: PNG + ICO sizes ${SIZES.join("/")} and reference`);
