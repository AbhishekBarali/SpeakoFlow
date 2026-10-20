/**
 * The flowing bands behind the share cards and the Insights summary: the
 * layered waves of the SpeakoFlow icon, drawn as a ribbon of tonal bands that
 * fans out as it crosses the frame. One geometry serves the canvas and SVG,
 * so the page and the cards are visibly the same mark.
 */

export type Point = [number, number];

export interface FlowSpec {
  /** The ribbon's spine, a cubic Bézier in fractions of the frame: x of its
   *  width, y of its height. It runs past the edges, so the frame crops it. */
  spine: [Point, Point, Point, Point];
  /** The edges of the bands, as distances from the spine in fractions of the
   *  frame's shorter side; n edges make n - 1 bands. */
  edges: number[];
  /** How wide the ribbon is at its start and at its end, as multiples of
   *  `edges`: a ribbon that widens reads as flowing out of the frame. */
  fan: [number, number];
  /** Fill on from the last edge to the frame's edge on that side, so the
   *  bands read as layers of a landscape rather than as one stroke. The
   *  spine must start and end outside the frame across that side. */
  fill?: "top" | "right" | "corner";
}

const SAMPLES = 180;

const at = (spine: FlowSpec["spine"], t: number): Point => {
  const u = 1 - t;
  const a = u * u * u;
  const b = 3 * u * u * t;
  const c = 3 * u * t * t;
  const d = t * t * t;
  return [
    a * spine[0][0] + b * spine[1][0] + c * spine[2][0] + d * spine[3][0],
    a * spine[0][1] + b * spine[1][1] + c * spine[2][1] + d * spine[3][1],
  ];
};

/** The curve of each edge, in pixels, sampled along the spine. */
const edgeCurves = (spec: FlowSpec, w: number, h: number): Point[][] => {
  const side = Math.min(w, h);
  const spine = spec.spine.map(([x, y]) => [x * w, y * h]) as FlowSpec["spine"];
  const points: Point[] = [];
  for (let i = 0; i <= SAMPLES; i += 1) points.push(at(spine, i / SAMPLES));
  return spec.edges.map((edge) =>
    points.map(([x, y], i) => {
      const before = points[Math.max(0, i - 1)];
      const after = points[Math.min(SAMPLES, i + 1)];
      const dx = after[0] - before[0];
      const dy = after[1] - before[1];
      const length = Math.hypot(dx, dy) || 1;
      const t = i / SAMPLES;
      // Smoothstep, so the ribbon widens gently rather than as a wedge.
      const eased = t * t * (3 - 2 * t);
      const fan = spec.fan[0] + (spec.fan[1] - spec.fan[0]) * eased;
      const offset = edge * side * fan;
      return [x - (dy / length) * offset, y + (dx / length) * offset];
    }),
  );
};

/** Each band as a closed outline: one edge out, the next edge back. */
export const flowBands = (spec: FlowSpec, w: number, h: number): Point[][] => {
  const curves = edgeCurves(spec, w, h);
  const bands: Point[][] = [];
  for (let i = 0; i < curves.length - 1; i += 1) {
    bands.push([...curves[i], ...curves[i + 1].slice().reverse()]);
  }
  if (spec.fill) {
    // The far side of the last edge, closed well outside the frame.
    const last = curves[curves.length - 1];
    const start = last[0];
    const end = last[last.length - 1];
    const far = Math.max(w, h) * 4;
    const close: Point[] =
      spec.fill === "top"
        ? [
            [end[0], -far],
            [start[0], -far],
          ]
        : spec.fill === "right"
          ? [
              [far, end[1]],
              [far, start[1]],
            ]
          : [
              [far, end[1]],
              [far, far],
              [start[0], far],
            ];
    bands.push([...last, ...close]);
  }
  return bands;
};

/** A band as SVG path data. */
export const bandPath = (band: Point[]): string =>
  `M${band.map(([x, y]) => `${x.toFixed(1)} ${y.toFixed(1)}`).join("L")}Z`;
