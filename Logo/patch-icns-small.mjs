// Patch the small entries of src-tauri/icons/icon.icns to the small tier.
//
// Tauri's icon generator writes one artwork into every entry, so macOS gets the
// hero art at 16 and 32px too -- exactly the sizes it cannot survive (Finder list
// and column views, the "get info" strip). The large PNG entries are left alone.
//
// The 16 and 32px entries are the legacy pair formats rather than PNG:
//   is32 / il32  RGB, channel-planar, RLE24-compressed
//   s8mk / l8mk  8-bit alpha, uncompressed
// so they are re-encoded here. The RLE24 encoder is verified by decoding its own
// output and comparing to the input before anything is written -- there is no way
// to test an .icns from Windows, so lossless round-trip is the bar.
//
// Run from the Logo folder: node patch-icns-small.mjs

import fs from "node:fs";
import path from "node:path";
import sharp from "sharp";
import { smallIconSvg } from "./small-icon.mjs";

const ICNS = path.join("..", "src-tauri", "icons", "icon.icns");

// Rendered at the target size so the small tier's pixel snapping applies.
const renderSmall = (size) =>
  sharp(Buffer.from(smallIconSvg(size)), { density: 384 })
    .resize(size, size)
    .png({ compressionLevel: 9 })
    .toBuffer();

/** ICNS RLE24: per channel, 0x00-0x7F = literal run of n+1, 0x80-0xFF = repeat n-125. */
function encodeRle24(planes) {
  const out = [];
  for (const plane of planes) {
    let i = 0;
    while (i < plane.length) {
      // Longest run of identical bytes at i (3..130 encodes as one op).
      let run = 1;
      while (run < 130 && i + run < plane.length && plane[i + run] === plane[i])
        run++;
      if (run >= 3) {
        out.push(run + 125, plane[i]);
        i += run;
        continue;
      }
      // Otherwise gather a literal run, stopping before a run of 3+.
      let lit = 0;
      while (lit < 128 && i + lit < plane.length) {
        const a = plane[i + lit];
        if (
          i + lit + 2 < plane.length &&
          plane[i + lit + 1] === a &&
          plane[i + lit + 2] === a
        )
          break;
        lit++;
      }
      if (lit === 0) lit = 1;
      out.push(lit - 1);
      for (let k = 0; k < lit; k++) out.push(plane[i + k]);
      i += lit;
    }
  }
  return Buffer.from(out);
}

/** Inverse of encodeRle24, used only to prove the encoder is lossless. */
function decodeRle24(buf, planeLen, planeCount) {
  const planes = [];
  let i = 0;
  for (let p = 0; p < planeCount; p++) {
    const plane = Buffer.alloc(planeLen);
    let w = 0;
    while (w < planeLen) {
      if (i >= buf.length) throw new Error("RLE24 truncated");
      const ctl = buf[i++];
      if (ctl & 0x80) {
        const n = ctl - 125;
        const v = buf[i++];
        for (let k = 0; k < n && w < planeLen; k++) plane[w++] = v;
      } else {
        const n = ctl + 1;
        for (let k = 0; k < n && w < planeLen; k++) plane[w++] = buf[i++];
      }
    }
    planes.push(plane);
  }
  return planes;
}

async function planesFor(size) {
  const { data } = await sharp(await renderSmall(size))
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  const n = size * size;
  const r = Buffer.alloc(n),
    g = Buffer.alloc(n),
    b = Buffer.alloc(n),
    a = Buffer.alloc(n);
  for (let i = 0; i < n; i++) {
    r[i] = data[i * 4];
    g[i] = data[i * 4 + 1];
    b[i] = data[i * 4 + 2];
    a[i] = data[i * 4 + 3];
  }
  return { planes: [r, g, b], alpha: a, n };
}

// Parse the existing container.
const src = fs.readFileSync(ICNS);
if (src.subarray(0, 4).toString("ascii") !== "icns")
  throw new Error("not an icns");
const entries = [];
let off = 8;
while (off + 8 <= src.length) {
  const type = src.subarray(off, off + 4).toString("ascii");
  const len = src.readUInt32BE(off + 4);
  if (len < 8 || off + len > src.length)
    throw new Error(`bad entry ${type} at ${off}`);
  entries.push({ type, data: src.subarray(off + 8, off + len) });
  off += len;
}
console.log(
  `read ${path.basename(ICNS)}: ${entries.length} entries, ${src.length}b`,
);

// Build the replacements.
const replacements = new Map();
for (const [size, rgbType, maskType] of [
  [16, "is32", "s8mk"],
  [32, "il32", "l8mk"],
]) {
  const { planes, alpha, n } = await planesFor(size);
  const rle = encodeRle24(planes);
  const back = decodeRle24(rle, n, 3);
  for (let p = 0; p < 3; p++) {
    if (!back[p].equals(planes[p])) {
      throw new Error(`RLE24 round-trip FAILED for ${rgbType} plane ${p}`);
    }
  }
  console.log(
    `  ${rgbType}: ${size}x${size} RGB -> ${rle.length}b RLE24, round-trip lossless OK`,
  );
  replacements.set(rgbType, rle);
  replacements.set(maskType, alpha);
}
// The PNG entries that are still small enough to need the simplified mark.
replacements.set("ic11", await renderSmall(32)); // 16@2x
replacements.set("ic12", await renderSmall(64)); // 32@2x
console.log("  ic11 (32px) and ic12 (64px) PNG entries -> small tier");

// Reassemble, preserving entry order so nothing about the container changes but
// the payloads we mean to change.
const chunks = [];
let replaced = 0;
for (const e of entries) {
  const data = replacements.get(e.type) ?? e.data;
  if (replacements.has(e.type)) replaced++;
  const head = Buffer.alloc(8);
  head.write(e.type, 0, 4, "ascii");
  head.writeUInt32BE(data.length + 8, 4);
  chunks.push(head, data);
}
const body = Buffer.concat(chunks);
const header = Buffer.alloc(8);
header.write("icns", 0, 4, "ascii");
header.writeUInt32BE(body.length + 8, 4);
const outBuf = Buffer.concat([header, body]);
fs.writeFileSync(ICNS, outBuf);
console.log(
  `wrote ${path.basename(ICNS)}: ${replaced} entries replaced, ${outBuf.length}b` +
    ` (was ${src.length}b)`,
);

// Re-parse what we just wrote, so a malformed container cannot ship silently.
const check = fs.readFileSync(ICNS);
if (check.readUInt32BE(4) !== check.length)
  throw new Error("declared length mismatch");
let o = 8,
  seen = 0;
while (o + 8 <= check.length) {
  const type = check.subarray(o, o + 4).toString("ascii");
  const len = check.readUInt32BE(o + 4);
  if (len < 8 || o + len > check.length)
    throw new Error(`bad entry ${type} after write`);
  if (!/^[a-zA-Z0-9]{4}$/.test(type))
    throw new Error(`bad type tag ${JSON.stringify(type)}`);
  seen++;
  o += len;
}
if (o !== check.length) throw new Error("trailing bytes");
console.log(
  `verified: ${seen} entries parse cleanly, declared length matches file`,
);
