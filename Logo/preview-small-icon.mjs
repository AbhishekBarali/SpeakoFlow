// Actual-size review on Windows-like dark and light surfaces.
// Run from Logo: node preview-small-icon.mjs
import fs from "node:fs";
import sharp from "sharp";
import { renderSmall } from "./small-icon.mjs";

const OUT = "final-v2/preview";
// Preserve the first generated version to show the actual optical correction.
async function previous(size) {
  let image = sharp(`${OUT}/before-optical-fix.png`).resize(size, size, {
    kernel: "lanczos3",
  });
  if (size <= 48) image = image.sharpen({ sigma: 0.55, m1: 0, m2: 1.3 });
  return image.png().toBuffer();
}
fs.mkdirSync(OUT, { recursive: true });
const columns = [
  {
    x: 40,
    label: "Original detailed artwork",
    src: "final-v2/icon-master-2048.png",
  },
  { x: 352, label: "First generated version", previous: true },
  { x: 664, label: "Restored, refined rendering", previous: false },
];
const sizes = [16, 20, 24, 32, 40, 48, 64];
const layers = [];
const labels = [];
for (const column of columns) {
  labels.push(`<text x="${column.x}" y="46">${column.label}</text>`);
  const large = column.previous
    ? await previous(160)
    : column.src
      ? await sharp(column.src).resize(160, 160).png().toBuffer()
      : await renderSmall(160);
  layers.push({ input: large, left: column.x, top: 72 });
  let x = column.x;
  for (const size of sizes) {
    const icon = column.previous
      ? await previous(size)
      : column.src
        ? await sharp(column.src).resize(size, size).png().toBuffer()
        : await renderSmall(size);
    layers.push({
      input: icon,
      left: x,
      top: 300 + Math.round((64 - size) / 2),
    });
    layers.push({
      input: icon,
      left: x,
      top: 450 + Math.round((64 - size) / 2),
    });
    labels.push(
      `<text x="${x + size / 2}" y="392" text-anchor="middle" font-size="11">${size}</text>`,
    );
    x += size + 8;
  }
}
const background = Buffer.from(
  `<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="600"><rect width="1000" height="600" fill="#202020"/><rect y="422" width="1000" height="124" fill="#f5f5f5"/><g fill="#eee" font-family="Segoe UI, sans-serif" font-size="17">${labels.join("")}<text x="40" y="274" font-size="13" fill="#aaa">Actual pixel sizes (100% display scale). Large images above show the artwork.</text><text x="40" y="580" font-size="13" fill="#aaa">Original waveform and flow preserved. Only reduction filtering changes.</text></g></svg>`,
);
await sharp(background)
  .composite(layers)
  .png()
  .toFile(`${OUT}/small-icon-comparison.png`);
console.log(`Saved ${OUT}/small-icon-comparison.png`);
