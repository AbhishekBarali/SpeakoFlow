/**
 * RETIRED (kept for reference). This builds the PREVIOUS SpeakoFlow Mini badge —
 * the old speech-bubble mark with an eraser across the corner. The badge now
 * uses the current app tile and is written by apply-icon-to-app.mjs; running
 * this script would put the old artwork back into src/assets/.
 *
 * Build the app-bundled SpeakoFlow Mini mark from the full-resolution artwork.
 *
 * The source is a ~1083x1169 transparent PNG with a wide dead margin, far too
 * heavy (840 KB) for a 40px catalog tile. This trims the transparent border so
 * the mark fills its tile, then emits a 128px asset (3x headroom for the
 * largest tile we draw) into `src/assets/`.
 *
 * Run from the Logo folder: `node build-mini-mark.mjs`
 */
import sharp from "sharp";
import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const SRC = resolve("Final Transparent Logo speakoflow mini .png");
const OUT = resolve("../src/assets/speakoflow-mini.png");
const EDGE = 128;

await mkdir(dirname(OUT), { recursive: true });

const trimmed = await sharp(SRC)
  .trim({ threshold: 1 })
  .toBuffer({ resolveWithObject: true });

await sharp(trimmed.data)
  .resize({
    width: EDGE,
    height: EDGE,
    fit: "contain",
    background: { r: 0, g: 0, b: 0, alpha: 0 },
  })
  .png({ compressionLevel: 9, palette: false })
  .toFile(OUT);

const out = await sharp(OUT).metadata();
console.log(
  `trimmed ${trimmed.info.width}x${trimmed.info.height} -> ${out.width}x${out.height} (${out.size} bytes)`,
);
