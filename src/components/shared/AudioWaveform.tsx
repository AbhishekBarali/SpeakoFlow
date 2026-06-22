import React, { useRef } from "react";
import "./AudioWaveform.css";

export interface AudioWaveformProps {
  /** Raw vocal-spectrum levels (roughly 0..1), any length. */
  levels: number[];
  /** Number of bars to render. Defaults to a sensible value per `size`. */
  barCount?: number;
  /** Whether audio is actively flowing. When false the bars rest as dots. */
  active?: boolean;
  /** Visual scale — `sm` for the compact overlay, `md` for the pill / panel. */
  size?: "sm" | "md";
  className?: string;
}

const SIZES = {
  sm: { barW: 2.5, gap: 2, bars: 19 },
  md: { barW: 3, gap: 2, bars: 27 },
} as const;

// Resting dot vs. tallest bar, as a share of the container height. Heights are
// container-relative so one component fills a short overlay chip or a taller
// pill without per-call tuning.
const MIN_PCT = 9;
const MAX_PCT = 96;

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
 * A mirrored "voice spindle" waveform. The backend streams a 16-band vocal
 * spectrum; we fold it symmetrically so the loud low-mid bands sit in the
 * centre and ease out to the quiet highs at both edges, then shape it with a
 * bell envelope so it always resolves to the elegant centre-weighted
 * silhouette of a voice burst — tall in the middle, settling to small dots at
 * the rim. Levels are smoothed frame-to-frame so motion stays fluid.
 *
 * Bar heights are a share of the container height and the bars flex to fill the
 * available width, so the same component looks right in both the short
 * recording overlay and the taller assistant pill.
 */
const AudioWaveform: React.FC<AudioWaveformProps> = ({
  levels,
  barCount,
  active = true,
  size = "sm",
  className = "",
}) => {
  const { barW, gap, bars: defaultBars } = SIZES[size];
  const count = barCount ?? defaultBars;

  // Unique bands from the centre (low freq, loud) out to one edge.
  const half = Math.max(2, Math.ceil(count / 2));
  const smoothedRef = useRef<number[]>(new Array(half).fill(0));
  if (smoothedRef.current.length !== half) {
    smoothedRef.current = new Array(half).fill(0);
  }

  const profile = resample(levels, half);
  const display = smoothedRef.current.map(
    (prev, i) => prev * 0.6 + (profile[i] ?? 0) * 0.4,
  );
  smoothedRef.current = display;

  const energy = display.reduce((m, v) => Math.max(m, v), 0);
  const idle = !active || energy < 0.05;
  const center = (count - 1) / 2;

  return (
    <div
      className={`audio-waveform ${size} ${idle ? "is-idle" : ""} ${className}`}
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
          const env = Math.pow(
            0.5 + 0.5 * Math.cos(Math.PI * Math.min(1, dist)),
            1.4,
          );
          const band = display[Math.round(dist * (half - 1))] ?? 0;
          const shaped = Math.pow(Math.min(1, Math.max(0, band)), 0.6);
          const amp = idle ? 0 : shaped * env;
          const pct = MIN_PCT + amp * (MAX_PCT - MIN_PCT);
          return (
            <span
              key={i}
              className="wave-bar"
              style={
                {
                  height: `${pct}%`,
                  "--i": i,
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
