// Verify the new brand icon actually reached every app asset.
//
// The two artworks are easy to tell apart without eyeballing: the OLD tile is a
// flat #14b8a6 field with a pure-white mark, so its distinct-colour count is
// tiny and it has no dark pixels; the NEW tile is a gradient running from pale
// mint to deep teal (~#005666). So an asset is "new" if it carries a real
// gradient AND a dark-teal region, and "OLD" if it is dominated by one flat hue.
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

async function classify(file) {
  const { data, info } = await sharp(file)
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  const { width: W, height: H, channels: C } = info;
  const colors = new Set();
  let flatTeal = 0,
    darkTeal = 0,
    ink = 0,
    opaque = 0;
  for (let i = 0; i < W * H; i++) {
    if (data[i * C + 3] < 128) continue;
    opaque++;
    const px = { r: data[i * C], g: data[i * C + 1], b: data[i * C + 2] };
    colors.add((px.r >> 3) * 1024 + (px.g >> 3) * 32 + (px.b >> 3));
    if (near(px, OLD_TEAL, 6)) flatTeal++;
    // Deep teal shadow that only exists in the new gradient tile.
    if (px.b > 40 && px.b < 130 && px.g > 60 && px.g < 160 && px.r < 60)
      darkTeal++;
    if (px.r < 40 && px.g < 40 && px.b < 40) ink++;
  }
  const flatShare = flatTeal / Math.max(1, opaque);
  const darkShare = darkTeal / Math.max(1, opaque);
  const verdict =
    flatShare > 0.25
      ? "OLD (flat teal)"
      : darkShare > 0.02 && colors.size > 200
        ? "new"
        : ink / Math.max(1, opaque) > 0.5
          ? "mono glyph"
          : "??";
  return {
    dims: `${W}x${H}`,
    colors: colors.size,
    flat: flatShare,
    dark: darkShare,
    verdict,
  };
}

const APP = [
  // Tauri app / installer / store / mobile
  "src-tauri/icons/icon.png",
  "src-tauri/icons/32x32.png",
  "src-tauri/icons/64x64.png",
  "src-tauri/icons/128x128.png",
  "src-tauri/icons/128x128@2x.png",
  "src-tauri/icons/StoreLogo.png",
  "src-tauri/icons/Square44x44Logo.png",
  "src-tauri/icons/Square310x310Logo.png",
  "src-tauri/icons/android/mipmap-xxxhdpi/ic_launcher.png",
  "src-tauri/icons/ios/AppIcon-512@2x.png",
  // Tray + taskbar
  "src-tauri/resources/tray_idle.png",
  "src-tauri/resources/tray_idle_dark.png",
  "src-tauri/resources/speakoflow.png",
  "src-tauri/resources/window_icon_light.png",
  "src-tauri/resources/window_icon_dark.png",
  // Frontend
  "src/assets/speakoflow-mini.png",
  // Docs site
  "docs-site/favicon.png",
  // Named deliverables
  "Logo/Final logo.png",
];

// Intentionally untouched: recording/transcribing STATE glyphs.
const KEEP = [
  "src-tauri/resources/tray_recording.png",
  "src-tauri/resources/tray_recording_dark.png",
  "src-tauri/resources/tray_transcribing.png",
  "src-tauri/resources/tray_transcribing_dark.png",
  "src-tauri/resources/recording.png",
  "src-tauri/resources/transcribing.png",
];

let bad = 0;
console.log("== app assets (expect: new) ==");
for (const f of APP) {
  const r = await classify(f);
  const ok = r.verdict === "new";
  if (!ok) bad++;
  console.log(
    `${ok ? "OK  " : "FAIL"} ${f.padEnd(52)} ${r.dims.padEnd(10)} colors=${String(r.colors).padStart(4)} flatTeal=${(r.flat * 100).toFixed(0)}% deepTeal=${(r.dark * 100).toFixed(0)}%  ${r.verdict}`,
  );
}

console.log("\n== state glyphs (expect: unchanged / not the brand tile) ==");
for (const f of KEEP) {
  const r = await classify(f);
  console.log(
    `--   ${f.padEnd(52)} ${r.dims.padEnd(10)} colors=${String(r.colors).padStart(4)} ${r.verdict}`,
  );
}

// Binary containers: check the ICO/ICNS were rewritten and still parse.
console.log("\n== containers ==");
const ico = fs.readFileSync("src-tauri/icons/icon.ico");
console.log(
  `icon.ico   ${ico.length}b, ${ico.readUInt16LE(4)} entries, magic=${ico.readUInt16LE(0)}/${ico.readUInt16LE(2)}`,
);
const icns = fs.readFileSync("src-tauri/icons/icon.icns");
console.log(
  `icon.icns  ${icns.length}b, magic=${icns.subarray(0, 4).toString("ascii")}, declared=${icns.readUInt32BE(4)}`,
);
// The largest PNG inside icon.ico should match the new artwork too.
const entries = [];
for (let i = 0; i < ico.readUInt16LE(4); i++) {
  const off = 6 + i * 16;
  entries.push({
    w: ico[off] || 256,
    size: ico.readUInt32LE(off + 8),
    at: ico.readUInt32LE(off + 12),
  });
}
entries.sort((a, b) => b.w - a.w);
const biggest = entries[0];
const tmp = path.join("Logo", ".ico-largest.png");
fs.writeFileSync(tmp, ico.subarray(biggest.at, biggest.at + biggest.size));
const inner = await classify(tmp);
fs.rmSync(tmp);
console.log(
  `icon.ico largest entry (${biggest.w}px): colors=${inner.colors} deepTeal=${(inner.dark * 100).toFixed(0)}% -> ${inner.verdict}`,
);
if (inner.verdict !== "new") bad++;

// Text references: no source file should still point at the retired lockups.
console.log("\n== references ==");
const refs = [
  ["README.md", /Logo\/final-v2\/png\/lockup(-dark)?-h256\.png/g, 2],
  ["public/favicon.svg", /data:image\/png;base64,/g, 1],
  ["docs-site/logo/lockup.svg", /data:image\/png;base64,/g, 1],
  ["docs-site/logo/lockup-dark.svg", /data:image\/png;base64,/g, 1],
  ["docs-site/logo/icon.svg", /data:image\/png;base64,/g, 1],
];
for (const [file, re, want] of refs) {
  const got = (fs.readFileSync(file, "utf8").match(re) ?? []).length;
  const ok = got >= want;
  if (!ok) bad++;
  console.log(`${ok ? "OK  " : "FAIL"} ${file.padEnd(34)} ${re} -> ${got}`);
}
const stale = ["README.md", "index.html"].flatMap((f) => {
  const s = fs.readFileSync(f, "utf8");
  return /Logo\/final\/(lockup|png\/icon)/.test(s) ? [f] : [];
});
console.log(
  stale.length
    ? `FAIL stale Logo/final refs in: ${stale}`
    : "OK   no stale Logo/final/ references",
);
bad += stale.length;

console.log(bad === 0 ? "\nALL CHECKS PASSED" : `\n${bad} CHECK(S) FAILED`);
process.exit(bad === 0 ? 0 : 1);
