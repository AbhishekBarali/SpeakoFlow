import React, { useRef } from "react";
import "./AudioWaveform.css";

export type WaveMode = "reactive" | "shimmer" | "flow";

export interface AudioWaveformProps {
  /** Raw vocal-spectrum levels (roughly 0..1), any length. */
  levels: number[];
  /** Number of bars to render. */
  barCount?: number;
  /** Whether audio is actively flowing. When false the bars rest as dots.
   *  Only consulted in `reactive` mode. */
  active?: boolean;
  /** Motion style. `reactive` follows `levels` (live mic); `shimmer` is a calm
   *  self-animated pulse for "working"/"thinking"; `flow` is a livelier
   *  self-animated pulse for "speaking". Defaults to `reactive`. */
  mode?: WaveMode;
  /** Visual scale — `sm` for the compact overlay, `md` for the pill / panel. */
  size?: "sm" | "md";
  className?: string;
}

const SIZES = {
  sm: { barW: 2.5, gap: 2, bars: 19 },
  md: { barW: 3.5, gap: 3, bars: 20 },
} as const;

// Resting dot vs. tallest bar, as a share of the container height.
const MIN_PCT = 9;
const MAX_PCT = 96;

// Resting silhouette height for the self-animated states, as a share of the
// reactive range — `flow` (speaking) stands taller and livelier than the calm
// `shimmer` (thinking). CSS then pulses each bar around this baseline.
const SELF_SILHOUETTE: Record<Exclude<WaveMode, "reactive">, number> = {
  shimmer: 0.5,
  flow: 0.82,
};

/** Centre-weighted bell envelope: 1 at the centre line, easing to 0 at the rim
 *  so the bars always read as a voice "spindle" rather than a flat row. */
function bell(dist: number): number {
  return Math.pow(0.5 + 0.5 * Math.cos(Math.PI * Math.min(1, dist)), 1.4);
}

/** Resample an arbitrary-length level array to exactly `n` points with linear
 *  interpolation, so the shape is stable regardless of how many bands the
 *  backend happens to send. */
function resample(src: number[], n: number): number[] {
  if (src.length === 0) return new Array(n).fill(0);
  if (src.length === n) return src.slice();
  const out = new Array<number>(n);
  for (let i = 0; i < n; i++) {
    const t = n === 1 ? 0 : (i / (n - 1)) * (src.length - 1);
    const lo = Math.floor(t);
    const hi = Math.ceil(t);
    const frac = t - lo;
    const a = src[lo] ?? 0;
    const b = src[hi] ?? a;
    out[i] = a * (1 - frac) + b * frac;
  }
  return out;
}

/**
 * Voice waveform rendered as a single, consistent visual language: a mirrored
 * "voice spindle" of rounded-cap bars that arch up from a centre line.
 *
 * - **reactive** (listening): bar heights are driven by the live 16-band vocal
 *   spectrum, smoothed frame-to-frame.
 * - **shimmer / flow** (working / speaking): the bars hold a calm spindle
 *   silhouette and pulse with a centre-out ripple driven entirely in CSS — no
 *   live audio, but the same crisp bars, so motion always reads as "voice"
 *   rather than a decorative line.
 *
 * Heights / sizes are container-relative, so the same component fills the short
 * recording overlay and the taller assistant pill alike.
 */
const AudioWaveform: React.FC<AudioWaveformProps> = ({
  levels,
  barCount,
  active = true,
  mode = "reactive",
  size = "sm",
  className = "",
}) => {
  const { barW, gap, bars: defaultBars } = SIZES[size];
  const count = barCount ?? defaultBars;
  const center = (count - 1) / 2;
  const isReactive = mode === "reactive";

  // Unique bands from the centre (low freq, loud) out to one edge.
  const half = Math.max(2, Math.ceil(count / 2));
  const smoothedRef = useRef<number[]>(new Array(half).fill(0));
  if (smoothedRef.current.length !== half) {
    smoothedRef.current = new Array(half).fill(0);
  }

  // Reactive mode smooths the incoming spectrum frame-to-frame; the
  // self-animated modes ignore `levels` entirely (motion comes from CSS).
  let display: number[] = smoothedRef.current;
  let idle = false;
  if (isReactive) {
    const profile = resample(levels, half);
    display = smoothedRef.current.map(
      (prev, i) => prev * 0.6 + (profile[i] ?? 0) * 0.4,
    );
    smoothedRef.current = display;
    const energy = display.reduce((m, v) => Math.max(m, v), 0);
    idle = !active || energy < 0.05;
  }

  const silhouette = isReactive ? 0 : SELF_SILHOUETTE[mode];

  return (
    <div
      className={`audio-waveform ${size} ${mode} ${
        isReactive && idle ? "is-idle" : ""
      } ${className}`}
      style={
        {
          "--bar-gap": `${gap}px`,
          "--bar-w": `${barW}px`,
        } as React.CSSProperties
      }
      aria-hidden="true"
    >
      <div className="wave-bars">
        {Array.from({ length: count }, (_, i) => {
          // 0 at the centre line, 1 at the outer edge.
          const dist = center === 0 ? 0 : Math.abs(i - center) / center;
          // Bell envelope: the centre carries the signal, the rim eases to dots.
          const env = bell(dist);

          if (isReactive) {
            const band = display[Math.round(dist * (half - 1))] ?? 0;
            const shaped = Math.pow(Math.min(1, Math.max(0, band)), 0.6);
            const amp = idle ? 0 : shaped * env;
            const pct = MIN_PCT + amp * (MAX_PCT - MIN_PCT);
            return (
              <span
                key={i}
                className="wave-bar"
                style={{ height: `${pct}%` } as React.CSSProperties}
              />
            );
          }

          // Self-animated: a static spindle silhouette that CSS pulses around.
          // `--dist` phases the centre-out ripple; the height is the resting
          // shape, scaled by the per-mode silhouette weight.
          const pct = MIN_PCT + env * silhouette * (MAX_PCT - MIN_PCT);
          return (
            <span
              key={i}
              className="wave-bar"
              style={
                {
                  height: `${pct}%`,
                  "--dist": dist.toFixed(3),
                } as React.CSSProperties
              }
            />
          );
        })}
      </div>
    </div>
  );
};

export default AudioWaveform;
