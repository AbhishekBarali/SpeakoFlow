// Verify the generated icon set: full-bleed geometry, clean alpha silhouette,
// and a corner radius matching the reference (old) icon.
import fs from "node:fs";
import path from "node:path";
import sharp from "sharp";

async function probe(file, label) {
  const { data, info } = await sharp(file)
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  const { width: W, height: H, channels: C } = info;
  const A = (x, y) => data[(y * W + x) * C + 3];

  let minX = W,
    minY = H,
    maxX = -1,
    maxY = -1,
    semi = 0,
    opaque = 0;
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      const a = A(x, y);
      if (a >= 24) {
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
      if (a === 255) opaque++;
      else if (a > 0) semi++;
    }
  }

  // Corner radius: first row (from the top) whose left edge reaches x=0.
  let radius = -1;
  for (let y = 0; y < H && radius < 0; y++) if (A(0, y) >= 128) radius = y;

  const cy = Math.floor(H / 2);
  const cx = Math.floor(W / 2);
  console.log(
    `${label.padEnd(30)} ${String(W).padStart(4)}x${H}` +
      `  bbox=${minX},${minY}..${maxX},${maxY}` +
      `  bleed=${minX === 0 && minY === 0 && maxX === W - 1 && maxY === H - 1 ? "FULL" : "PADDED"}` +
      `  r≈${radius}(${(radius / W).toFixed(3)})` +
      `  edgeAlpha[L,T,R,B]=${[A(0, cy), A(cx, 0), A(W - 1, cy), A(cx, H - 1)].join(",")}` +
      `  corner(0,0)=${A(0, 0)}` +
      `  semiAA=${((semi / (W * H)) * 100).toFixed(1)}%`,
  );
  return { W, H, minX, minY, maxX, maxY, radius };
}

console.log("== reference: previous icon ==");
await probe("final/png/icon-1024.png", "final/png/icon-1024.png");
await probe("final/png/icon-256.png", "final/png/icon-256.png");

console.log("\n== source hand-off ==");
await probe("Speakoflow new Logo.png", "Speakoflow new Logo.png");

console.log("\n== new set ==");
const dir = "final-v2/png";
for (const f of fs
  .readdirSync(dir)
  .sort((a, b) => a.localeCompare(b, "en", { numeric: true })))
  await probe(path.join(dir, f), f);
await probe("final-v2/icon-master-2048.png", "icon-master-2048.png");

// ICO sanity: entry count + declared sizes.
const ico = fs.readFileSync("final-v2/speakoflow.ico");
const count = ico.readUInt16LE(4);
const entries = [];
for (let i = 0; i < count; i++) {
  const off = 6 + i * 16;
  entries.push(`${ico[off] || 256}x${ico[off + 1] || 256}`);
}
console.log(`\nspeakoflow.ico: ${count} entries -> ${entries.join(", ")}`);
