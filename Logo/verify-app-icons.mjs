// Verify the icon rollout: that every app asset carries the new brand, and that
// each one carries the RIGHT TIER for the size it is drawn at.
//
// Fingerprints, so this needs no eyeballing:
//   small tier -> contains pure white (#ffffff) pixels; the redrawn pills are
//                 pure white, and nothing in the hero art is
//   hero art   -> no pure white, but a deep-teal region (the gradient's dark
//                 corner) plus a wide colour count
//   old logo   -> a flat #14b8a6 field over a large share of the tile
//
// Run from the repo root: node Logo/verify-app-icons.mjs

import fs from "node:fs";
import path from "node:path";
import sharp from "sharp";

const OLD_TEAL = { r: 0x14, g: 0xb8, b: 0xa6 };
const near = (a, b, tol) =>
  Math.abs(a.r - b.r) <= tol &&
  Math.abs(a.g - b.g) <= tol &&
  Math.abs(a.b - b.b) <= tol;

async function classify(input) {
  const { data, info } = await sharp(input)
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  return classifyRaw(data, info.width, info.height, info.channels);
}

/**
 * Classify one ICO directory entry. Entries are stored as either PNG or a
 * headerless BMP (BITMAPINFOHEADER + bottom-up BGRA + AND mask); png-to-ico uses
 * BMP for the smaller sizes, which sharp cannot read directly.
 */
function classifyIcoEntry(buf) {
  if (
    buf.length > 8 &&
    buf[0] === 0x89 &&
    buf.subarray(1, 4).toString("ascii") === "PNG"
  ) {
    return { png: true };
  }
  const headerSize = buf.readUInt32LE(0);
  const W = buf.readInt32LE(4);
  const H = Math.abs(buf.readInt32LE(8)) / 2; // stored height covers XOR + AND masks
  const bits = buf.readUInt16LE(14);
  if (bits !== 32) throw new Error(`unsupported ICO BMP depth ${bits}`);
  const rgba = Buffer.alloc(W * H * 4);
  let src = headerSize;
  for (let y = H - 1; y >= 0; y--) {
    for (let x = 0; x < W; x++) {
      const d = (y * W + x) * 4;
      rgba[d] = buf[src + 2];
      rgba[d + 1] = buf[src + 1];
      rgba[d + 2] = buf[src];
      rgba[d + 3] = buf[src + 3];
      src += 4;
    }
  }
  return { png: false, ...classifyRaw(rgba, W, H, 4) };
}

function classifyRaw(data, W, H, C) {
  const colors = new Set();
  let flatTeal = 0,
    deepTeal = 0,
    pureWhite = 0,
    opaque = 0;
  for (let i = 0; i < W * H; i++) {
    if (data[i * C + 3] < 128) continue;
    opaque++;
    const px = { r: data[i * C], g: data[i * C + 1], b: data[i * C + 2] };
    colors.add((px.r >> 3) * 1024 + (px.g >> 3) * 32 + (px.b >> 3));
    if (near(px, OLD_TEAL, 6)) flatTeal++;
    if (px.r > 250 && px.g > 250 && px.b > 250) pureWhite++;
    if (px.b > 40 && px.b < 130 && px.g > 60 && px.g < 160 && px.r < 60)
      deepTeal++;
  }
  const share = (n) => n / Math.max(1, opaque);
  const tier =
    share(flatTeal) > 0.25
      ? "OLD"
      : share(pureWhite) > 0.01
        ? "small"
        : share(deepTeal) > 0.02 && colors.size > 150
          ? "hero"
          : "??";
  return {
    W,
    H,
    colors: colors.size,
    tier,
    white: share(pureWhite),
    deep: share(deepTeal),
  };
}

let bad = 0;
async function expect(file, want, label = file) {
  const r = await classify(file);
  const ok = r.tier === want;
  if (!ok) bad++;
  console.log(
    `${ok ? "OK  " : "FAIL"} ${label.padEnd(50)} ${`${r.W}x${r.H}`.padEnd(10)}` +
      ` tier=${r.tier.padEnd(5)} want=${want.padEnd(5)}` +
      ` white=${(r.white * 100).toFixed(0)}% deep=${(r.deep * 100).toFixed(0)}% colors=${r.colors}`,
  );
}

console.log("== small tier: everything an OS draws small ==");
for (const f of [
  "src-tauri/resources/tray_idle.png",
  "src-tauri/resources/tray_idle_dark.png",
  "src-tauri/resources/speakoflow.png",
  "src-tauri/resources/window_icon_light.png",
  "src-tauri/resources/window_icon_dark.png",
  "src-tauri/icons/32x32.png",
  "src-tauri/icons/64x64.png",
  "src-tauri/icons/Square30x30Logo.png",
  "src-tauri/icons/Square44x44Logo.png",
  "src-tauri/icons/StoreLogo.png",
])
  await expect(f, "small");

console.log("\n== hero art: large sizes only ==");
for (const f of [
  "src-tauri/icons/icon.png",
  "src-tauri/icons/128x128.png",
  "src-tauri/icons/128x128@2x.png",
  "src-tauri/icons/Square310x310Logo.png",
  "src/assets/speakoflow-mini.png",
  "docs-site/favicon.png",
  "Logo/Final logo.png",
])
  await expect(f, "hero");

console.log("\n== state glyphs: must stay untouched ==");
for (const f of [
  "src-tauri/resources/tray_recording.png",
  "src-tauri/resources/tray_transcribing.png",
  "src-tauri/resources/recording.png",
]) {
  const r = await classify(f);
  console.log(
    `--   ${f.padEnd(50)} ${`${r.W}x${r.H}`.padEnd(10)} colors=${r.colors} (flat glyph)`,
  );
  if (r.colors > 4) {
    console.log(`FAIL ${f} looks like artwork, not a flat state glyph`);
    bad++;
  }
}

console.log("\n== icon.ico: per-entry size and tier ==");
const ico = fs.readFileSync("src-tauri/icons/icon.ico");
const count = ico.readUInt16LE(4);
const entries = [];
for (let i = 0; i < count; i++) {
  const o = 6 + i * 16;
  entries.push({
    w: ico[o] || 256,
    size: ico.readUInt32LE(o + 8),
    at: ico.readUInt32LE(o + 12),
  });
}
entries.sort((a, b) => a.w - b.w);
console.log(`${count} entries: ${entries.map((e) => e.w).join(", ")}`);
const tmp = path.join("Logo", ".ico-entry.png");
for (const e of entries) {
  const payload = ico.subarray(e.at, e.at + e.size);
  const want = e.w <= 64 ? "small" : "hero";
  let r = classifyIcoEntry(payload);
  let kind = r.png ? "PNG" : "BMP";
  if (r.png) {
    fs.writeFileSync(tmp, payload);
    r = await classify(tmp);
    fs.rmSync(tmp);
  }
  const ok = r.tier === want;
  if (!ok) bad++;
  console.log(
    `${ok ? "OK  " : "FAIL"}   icon.ico @${String(e.w).padStart(3)}px ${kind}` +
      ` tier=${r.tier.padEnd(5)} want=${want.padEnd(5)}` +
      ` white=${(r.white * 100).toFixed(0)}% deep=${(r.deep * 100).toFixed(0)}% colors=${r.colors}`,
  );
}

console.log(
  "\n== icon.icns: entries parse, small entries are the small tier ==",
);
const icns = fs.readFileSync("src-tauri/icons/icon.icns");
if (
  icns.subarray(0, 4).toString("ascii") !== "icns" ||
  icns.readUInt32BE(4) !== icns.length
) {
  console.log("FAIL icns header/length invalid");
  bad++;
} else {
  let o = 8;
  const seen = [];
  while (o + 8 <= icns.length) {
    const type = icns.subarray(o, o + 4).toString("ascii");
    const len = icns.readUInt32BE(o + 4);
    if (len < 8 || o + len > icns.length) {
      console.log(`FAIL malformed icns entry ${type}`);
      bad++;
      break;
    }
    const data = icns.subarray(o + 8, o + len);
    const isPng = data.length > 8 && data[0] === 0x89;
    seen.push({ type, len, isPng, data });
    o += len;
  }
  console.log(`${seen.length} entries: ${seen.map((s) => s.type).join(", ")}`);
  for (const s of seen.filter((s) => ["ic11", "ic12"].includes(s.type))) {
    fs.writeFileSync(tmp, s.data);
    await expect(tmp, "small", `  icns ${s.type}`);
    fs.rmSync(tmp);
  }
  // is32/il32 are RLE24, not PNG -- just assert they are still the legacy format.
  for (const t of ["is32", "il32", "s8mk", "l8mk"]) {
    const e = seen.find((s) => s.type === t);
    const ok = e && !e.isPng;
    if (!ok) bad++;
    console.log(
      `${ok ? "OK  " : "FAIL"}   icns ${t} present as legacy raw (${e?.len ?? 0}b)`,
    );
  }
}

console.log("\n== favicon is vector now ==");
const fav = fs.readFileSync("public/favicon.svg", "utf8");
const vector = !fav.includes("base64") && fav.includes("<rect");
if (!vector) bad++;
console.log(
  `${vector ? "OK  " : "FAIL"} public/favicon.svg ${fs.statSync("public/favicon.svg").size}b,` +
    ` base64=${fav.includes("base64")}`,
);

console.log(bad === 0 ? "\nALL CHECKS PASSED" : `\n${bad} CHECK(S) FAILED`);
process.exit(bad === 0 ? 0 : 1);
