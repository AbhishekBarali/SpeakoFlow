// The SMALL-SIZE tier of the SpeakoFlow app icon, as vector.
//
// WHY IT EXISTS
// The hero artwork is a 9-element pale-mint waveform on a tile whose own
// highlight swirl peaks near rgb(163,252,244) -- within a couple of luminance
// points of the bars themselves. That reads beautifully at 128px and turns to
// mush below ~48px, which is every size an OS actually draws: taskbar, dock,
// tray, alt-tab, Explorer lists, favicons. It is not a resampling problem; an OS
// downscale and a perfect lanczos downscale look equally muddy (measured), so no
// filtering fixes it. The artwork itself has to change, which is why every
// serious icon family ships a simplified small tier.
//
// WHAT CHANGES
// Kept: the squircle at the same 0.225 corner radius, the waveform identity, and
// the palette -- the gradient stops are sampled from the hero art's own
// background, not invented.
// Changed, for legibility:
//   - the outermost two dots are dropped: below 32px they are sub-pixel and only
//     add mud, and 7 elements across 60% of 16px is under a pixel each
//   - the remaining 5 bars become pure white on one centre axis, spread wider and
//     made taller, so each still owns a whole pixel at 16px
//   - the tile is a clean two-stop gradient in the mid-to-deep part of the brand
//     range, so white has real contrast to sit on. The swirl is invisible at
//     these sizes anyway, so keeping it would only cost contrast
//
// PIXEL SNAPPING
// Geometry is rounded to whole pixels at the target size. Without it a 16px bar
// computes to ~1.2px wide, so every pixel is a blend and not one is solid white
// -- the classic reason a small icon looks smeared. Snapped, a 16px bar is a
// crisp 1px column. This is why callers must render at the size they need rather
// than scaling one raster down.

/**
 * Waveform geometry measured from Logo/final-v2/icon-master-2048.png by
 * connected-component analysis of its near-white, low-saturation pixels.
 * Fractions of the tile. `dot: true` marks the pair the small tier drops.
 */
export const MARK_ELEMENTS = [
  { cx: 0.21558, cy: 0.52344, w: 0.04102, h: 0.03857, dot: true },
  { cx: 0.2937, cy: 0.51758, w: 0.05957, h: 0.09717 },
  { cx: 0.3938, cy: 0.49585, w: 0.06348, h: 0.22168 },
  { cx: 0.49878, cy: 0.46362, w: 0.06738, h: 0.3291 },
  { cx: 0.5957, cy: 0.49731, w: 0.06201, h: 0.22461 },
  { cx: 0.69678, cy: 0.51953, w: 0.05908, h: 0.10107 },
  { cx: 0.77661, cy: 0.52393, w: 0.04102, h: 0.03955, dot: true },
];

export const CORNER = 0.225; // same silhouette as the hero tile
export const TILE_FROM = "#38c6c0"; // sampled from the hero background (TL region)
export const TILE_TO = "#04616e"; // ...and its deep corner (p05 of background luma)
export const BAR = "#ffffff";

const SPREAD = 1.34; // widen the gaps so bars stay separate at 16px
const WIDEN = 1.2; // thicken each bar
const HEIGHTEN = 1.45; // give the waveform real presence in the tile

const bars = MARK_ELEMENTS.filter((el) => !el.dot);
const groupCx = (bars[0].cx + bars[bars.length - 1].cx) / 2;
const groupW =
  bars[bars.length - 1].cx -
  bars[0].cx +
  (bars[0].w + bars[bars.length - 1].w) / 2;

/**
 * Lay the waveform out on whole pixels for `size`.
 * Bars get equal integer widths and gaps, so every edge falls on a pixel
 * boundary and the fill is solid rather than anti-aliased to grey.
 */
function snappedBars(size) {
  const idealW = bars[2].w * WIDEN * size; // the tallest bar sets the stroke weight
  const idealSpan = groupW * SPREAD * size;
  const idealGap = (idealSpan - bars.length * idealW) / (bars.length - 1);

  const barW = Math.max(1, Math.round(idealW));
  const gap = Math.max(1, Math.round(idealGap));
  const total = bars.length * barW + (bars.length - 1) * gap;
  const startX = Math.round((size - total) / 2);
  const mid = size / 2;

  return bars.map((el, i) => {
    const x = startX + i * (barW + gap);
    // Keep each bar's height ratio, but snap to an even/odd height that keeps the
    // bar centred on the same axis without a half-pixel top edge.
    let h = Math.max(1, Math.round(el.h * HEIGHTEN * size));
    const y = Math.round(mid - h / 2);
    if (y + h > size) h = size - y;
    // A 1-2px bar reads better square; wider bars keep the pill cap.
    const r = barW <= 2 ? 0 : barW / 2;
    return { x, y, w: barW, h, r };
  });
}

/** The small mark as an SVG string, snapped to the pixel grid of `size`. */
export function smallIconSvg(size) {
  const radius = size * CORNER;
  const pills = snappedBars(size)
    .map(
      (b) =>
        `    <rect x="${b.x}" y="${b.y}" width="${b.w}" height="${b.h}"` +
        (b.r ? ` rx="${b.r.toFixed(3)}" ry="${b.r.toFixed(3)}"` : "") +
        ` fill="${BAR}"/>`,
    )
    .join("\n");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">
  <title>SpeakoFlow</title>
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="${TILE_FROM}"/>
      <stop offset="1" stop-color="${TILE_TO}"/>
    </linearGradient>
    <radialGradient id="gloss" cx="0.3" cy="0.24" r="0.72">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.16"/>
      <stop offset="1" stop-color="#ffffff" stop-opacity="0"/>
    </radialGradient>
    <clipPath id="tileClip">
      <rect x="0" y="0" width="${size}" height="${size}" rx="${radius.toFixed(3)}" ry="${radius.toFixed(3)}"/>
    </clipPath>
  </defs>
  <g clip-path="url(#tileClip)">
    <rect x="0" y="0" width="${size}" height="${size}" fill="url(#tile)"/>
    <rect x="0" y="0" width="${size}" height="${size}" fill="url(#gloss)"/>
${pills}
  </g>
</svg>
`;
}
