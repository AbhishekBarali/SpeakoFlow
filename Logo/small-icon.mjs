// Preserve the approved first generated artwork, including its original bars.
// Refine resampling only; never replace or enlarge the waveform.
import sharp from "sharp";
import { fileURLToPath } from "node:url";

const SOURCE = fileURLToPath(
  new URL("./small-icon-source.png", import.meta.url),
);
const MASTER_SIZE = 1024;
const CORNER = 0.225;
let masterPromise;

async function normalizeMaster() {
  const { data, info } = await sharp(SOURCE)
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  const { width, height } = info;
  if (width !== height) throw new Error("Small icon source must be square");

  // The generated PNG has a baked checkerboard outside its rounded corners.
  // Extend each row's solid teal span, then apply a clean alpha silhouette.
  // Only the outside of each span changes; the generated curves stay intact.
  for (let y = 0; y < height; y++) {
    let first = -1;
    let last = -1;
    for (let x = 0; x < width; x++) {
      const p = (y * width + x) * 4;
      if (data[p + 1] - data[p] > 35 && data[p + 2] - data[p] > 25) {
        if (first < 0) first = x;
        last = x;
      }
    }
    if (first < 0) throw new Error(`No teal artwork on source row ${y}`);
    for (let x = 0; x < width; x++) {
      const p = (y * width + x) * 4;
      const edge = (y * width + Math.max(first, Math.min(last, x))) * 4;
      if (p !== edge) data.copy(data, p, edge, edge + 3);
      data[p + 3] = 255;
    }
  }

  const square = await sharp(data, { raw: { width, height, channels: 4 } })
    .resize(MASTER_SIZE, MASTER_SIZE, { kernel: "lanczos3" })
    .png()
    .toBuffer();
  const mask = Buffer.from(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${MASTER_SIZE}" height="${MASTER_SIZE}"><rect width="${MASTER_SIZE}" height="${MASTER_SIZE}" rx="${MASTER_SIZE * CORNER}" fill="white"/></svg>`,
  );
  return sharp(square)
    .composite([{ input: mask, blend: "dest-in" }])
    .png({ compressionLevel: 9 })
    .toBuffer();
}

export async function renderSmall(size) {
  masterPromise ??= normalizeMaster();
  // The reference remains pixel-identical to the version the user approved.
  if (size === MASTER_SIZE) return masterPromise;
  // Gamma-aware reduction preserves light strokes without redrawing them.
  // No unsharp filter: it produced bright fringes around small features.
  return sharp(await masterPromise)
    .gamma()
    .resize(size, size, { kernel: "lanczos3" })
    .png({ compressionLevel: 9 })
    .toBuffer();
}

// macOS menu-bar idle icon (src-tauri/resources/tray_idle_macos.png).
// The tray crate scales every menu-bar image to 18pt tall, so a 36px canvas is
// exactly @2x. The tile is the ordinary 32px small render with a 2px (1pt)
// transparent margin around it, which gives it the visual weight of the other
// menu-bar extras instead of filling the slot edge to edge. Nothing is redrawn:
// the margin is added around the same pixels every other small size uses.
export const MENU_BAR_SIZE = 36;
const MENU_BAR_TILE = 32;

export async function renderMenuBar() {
  const margin = (MENU_BAR_SIZE - MENU_BAR_TILE) / 2;
  return sharp(await renderSmall(MENU_BAR_TILE))
    .extend({
      top: margin,
      bottom: margin,
      left: margin,
      right: margin,
      background: { r: 0, g: 0, b: 0, alpha: 0 },
    })
    .png({ compressionLevel: 9 })
    .toBuffer();
}

// SVG-only consumers get a raster wrapper, not a different drawing.
export async function smallIconSvg(size = 64) {
  const png = await renderSmall(size);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}"><title>SpeakoFlow</title><image width="${size}" height="${size}" href="data:image/png;base64,${png.toString("base64")}"/></svg>\n`;
}
