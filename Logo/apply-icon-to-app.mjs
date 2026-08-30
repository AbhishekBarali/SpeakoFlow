// Roll the current brand icon out to every place the app draws its own logo.
//
// TWO TIERS, ONE CROSSOVER
// `final-v2` holds the hero artwork; `final-v2/small` holds the simplified mark
// (see build-small-icon.mjs for why it exists). The rule here is simply:
//
//     <= SMALL_MAX px  ->  small tier      > SMALL_MAX px  ->  hero art
//
// Everything an OS draws small -- taskbar, dock, tray, alt-tab, Explorer lists,
// favicons, Linux hicolor 32/64 -- takes the small tier, so no platform is left
// to downscale nine pale-mint elements into four pixels. Sizes above the
// crossover keep the hero art, where its gradient and swirl are the whole point.
//
// This is deliberately asset-side rather than per-platform code: handing every
// shell a legible bitmap fixes Windows, macOS and Linux at once, and needs no
// Win32 icon plumbing.
//
// src-tauri/icons/* comes from the Tauri CLI first:
//     bun run tauri icon Logo/final-v2/png/icon-1024.png
// then this script overwrites the small entries and rebuilds icon.ico mixed.
//
// Deliberately untouched: src-tauri/resources/tray_recording*.png,
// tray_transcribing*.png, recording.png, transcribing.png. Those are
// recording/transcribing STATE glyphs, not the brand mark.
//
// Run from the Logo folder: node apply-icon-to-app.mjs

import fs from "node:fs";
import path from "node:path";
import sharp from "sharp";
import pngToIco from "png-to-ico";
import { smallIconSvg } from "./small-icon.mjs";

const HERO_MASTER = path.join("final-v2", "icon-master-2048.png");
const REPO = "..";
const SMALL_MAX = 64; // crossover: at or below this, use the simplified mark

// Hero downscales get a light unsharp below 96px; the small tier never needs it
// because it is rendered from vector at the target size.
function hero(size) {
  let pipe = sharp(HERO_MASTER).resize(size, size, { kernel: "lanczos3" });
  if (size < 96) {
    pipe = pipe.sharpen({ sigma: 0.6 + 0.6 * (1 - size / 96), m1: 0, m2: 2.2 });
  }
  return pipe.png({ compressionLevel: 9 }).toBuffer();
}

// Rendered at the requested size, not scaled from a larger raster: the small tier
// snaps its geometry to the pixel grid of whatever size it is asked for.
const small = (size) =>
  sharp(Buffer.from(smallIconSvg(size)), { density: 384 })
    .resize(size, size)
    .png({ compressionLevel: 9 })
    .toBuffer();

const render = (size) => (size <= SMALL_MAX ? small(size) : hero(size));

// [destination, size, why]
const TARGETS = [
  // System tray. Windows draws 16, macOS's menu bar ~22, Linux 22-24 -- all well
  // inside the small tier, so these ship the simplified mark even at 64px source.
  ["src-tauri/resources/tray_idle.png", 64, "tray idle (dark themes)"],
  ["src-tauri/resources/tray_idle_dark.png", 64, "tray idle (light themes)"],
  ["src-tauri/resources/speakoflow.png", 64, "tray idle (Linux)"],
  // Taskbar / alt-tab / dock, set at runtime by tray.rs. Was 256px of hero art,
  // which the shell then crushed to 24px; 64px of the small tier is what the
  // shell can actually use.
  ["src-tauri/resources/window_icon_light.png", 64, "taskbar + alt-tab"],
  ["src-tauri/resources/window_icon_dark.png", 64, "taskbar + alt-tab"],
  // Linux hicolor theme + Tauri bundles (install-arch.sh installs 32 and 128).
  ["src-tauri/icons/32x32.png", 32, "Linux hicolor 32"],
  ["src-tauri/icons/64x64.png", 64, "Linux hicolor 64"],
  // Windows Store small tiles.
  ["src-tauri/icons/Square30x30Logo.png", 30, "Store tile 30"],
  ["src-tauri/icons/Square44x44Logo.png", 44, "Store tile 44"],
  ["src-tauri/icons/StoreLogo.png", 50, "Store logo 50"],
];

for (const [rel, size, why] of TARGETS) {
  const dest = path.join(REPO, rel);
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  fs.writeFileSync(dest, await render(size));
  console.log(
    `${rel.padEnd(46)} ${String(size).padStart(4)}px ${size <= SMALL_MAX ? "small" : "hero "}  ${fs.statSync(dest).size}b  (${why})`,
  );
}

// ---------------------------------------------------------------------------
// icon.ico: the .exe / installer / Explorer / pinned-taskbar icon.
// Mixed tiers in one container so Windows picks an exact entry at every size it
// asks for, instead of scaling a single large bitmap.
// ---------------------------------------------------------------------------
const ICO_SIZES = [16, 20, 24, 32, 48, 64, 128, 256];
const icoPath = path.join(REPO, "src-tauri/icons/icon.ico");
fs.writeFileSync(
  icoPath,
  await pngToIco(await Promise.all(ICO_SIZES.map((s) => render(s)))),
);
console.log(
  `${"src-tauri/icons/icon.ico".padEnd(46)}  ${ICO_SIZES.join("/")}  ${fs.statSync(icoPath).size}b (mixed tiers)`,
);

// ---------------------------------------------------------------------------
// The SpeakoFlow Mini badge keeps the hero art: it renders at 36-40px on a light
// card where the pale mint still reads, and 128px gives 3x headroom.
// ---------------------------------------------------------------------------
const miniPath = path.join(REPO, "src/assets/speakoflow-mini.png");
fs.mkdirSync(path.dirname(miniPath), { recursive: true });
fs.writeFileSync(miniPath, await hero(128));
console.log(
  `${"src/assets/speakoflow-mini.png".padEnd(46)}  128px hero   ${fs.statSync(miniPath).size}b  (model badge)`,
);

// ---------------------------------------------------------------------------
// favicon: the window entry points draw it at 16-32px, so it takes the small
// tier -- and since that tier is vector, the favicon is now real SVG rather than
// an embedded raster (92 KB -> ~1 KB, crisp at any size). Snapped on a 32px grid:
// a browser asking for 16 gets a clean 2:1 reduction of exact geometry.
// ---------------------------------------------------------------------------
const favicon = path.join(REPO, "public/favicon.svg");
fs.writeFileSync(favicon, smallIconSvg(32));
console.log(
  `${"public/favicon.svg".padEnd(46)}   vector small   ${fs.statSync(favicon).size}b  (window favicons)`,
);

// docs-site favicon stays hero: it is shown at 32px+ in a browser tab strip and
// beside the docs title, and the site's own lockup is hero art too.
const docsFavicon = path.join(REPO, "docs-site/favicon.png");
if (fs.existsSync(docsFavicon)) {
  fs.writeFileSync(docsFavicon, await hero(256));
  console.log(
    `${"docs-site/favicon.png".padEnd(46)}  256px hero   ${fs.statSync(docsFavicon).size}b  (docs site)`,
  );
}

console.log("\nDONE.");
