import React, { useRef } from "react";
import "./AudioWaveform.css";

export interface AudioWaveformProps {
  /** Raw amplitude levels (roughly 0..1), any length. Resampled to `barCount`. */
  levels: number[];
  /** Number of bars to render. */
  barCount?: number;
  /** Whether audio is actively flowing. When false the bars breathe gently. */
  active?: boolean;
  /** Visual scale — `sm` for the compact overlay pill, `md` for the panel. */
  size?: "sm" | "md";
  className?: string;
}

const SIZES = {
  sm: { minH: 3, maxH: 18, barW: 3, gap: 2.5 },
  md: { minH: 4, maxH: 26, barW: 3.5, gap: 3 },
} as const;

/** Resample an arbitrary-length level array to exactly `n` points with
 *  linear interpolation, so the waveform looks the same regardless of how
 *  many bands the backend happens to send. */
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
 * A modern, mirrored audio waveform. Bars grow symmetrically from the
 * centre line with rounded caps and a soft warm atmospheric bloom behind
 * them. Incoming levels are smoothed frame-to-frame so motion feels fluid,
 * and a gentle "breathing" animation plays whenever the signal is quiet.
 */
const AudioWaveform: React.FC<AudioWaveformProps> = ({
  levels,
  barCount = 15,
  active = true,
  size = "sm",
  className = "",
}) => {
  const { minH, maxH, barW, gap } = SIZES[size];
  const smoothedRef = useRef<number[]>(new Array(barCount).fill(0));

  // Keep the smoothing buffer in sync with the requested bar count.
  if (smoothedRef.current.length !== barCount) {
    smoothedRef.current = new Array(barCount).fill(0);
  }

  const target = resample(levels, barCount);
  const display = smoothedRef.current.map((prev, i) => {
    const next = prev * 0.6 + (target[i] ?? 0) * 0.4;
    return next;
  });
  smoothedRef.current = display;

  // Overall energy drives the warmth/intensity of the bloom behind the bars.
  const energy = display.reduce((m, v) => Math.max(m, v), 0);
  const idle = !active || energy < 0.04;

  return (
    <div
      className={`audio-waveform ${size} ${idle ? "is-idle" : ""} ${className}`}
      style={
        {
          "--bar-gap": `${gap}px`,
          "--wave-energy": idle ? 0.12 : Math.min(1, energy * 1.2),
        } as React.CSSProperties
      }
      aria-hidden="true"
    >
      <span className="wave-bloom" />
      <div className="wave-bars">
        {display.map((v, i) => {
          // Gentle centre-weighting so the middle bars arch a touch taller —
          // reads as more deliberate than a flat row.
          const window = 0.78 + 0.22 * Math.sin((Math.PI * i) / (barCount - 1));
          const shaped = Math.pow(Math.min(1, Math.max(0, v)), 0.7) * window;
          const h = minH + shaped * (maxH - minH);
          return (
            <span
              key={i}
              className="wave-bar"
              style={
                {
                  width: `${barW}px`,
                  "--i": i,
                  ...(idle ? {} : { height: `${h}px` }),
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
