import React, { useId, useRef } from "react";
import "./AudioWaveform.css";

export type WaveMode = "reactive" | "shimmer" | "flow";

export interface AudioWaveformProps {
  /** Raw vocal-spectrum levels (roughly 0..1), any length. */
  levels: number[];
  /** Number of bars to render (reactive mode only). */
  barCount?: number;
  /** Whether audio is actively flowing. When false the bars rest as dots.
   *  Only consulted in `reactive` mode. */
  active?: boolean;
  /** Motion style. `reactive` follows `levels` (live mic, rendered as bars);
   *  `shimmer` is a calm flowing ribbon for "working"/"thinking"; `flow` is a
   *  livelier flowing ribbon for "speaking". Defaults to `reactive`. */
  mode?: WaveMode;
  /** Visual scale — `sm` for the compact overlay, `md` for the pill / panel. */
  size?: "sm" | "md";
  className?: string;
}

const SIZES = {
  sm: { barW: 2.5, gap: 2, bars: 19 },
  md: { barW: 3.5, gap: 3, bars: 20 },
} as const;

// Resting dot vs. tallest bar, as a share of the container height (reactive).
const MIN_PCT = 9;
const MAX_PCT = 96;

// --- Smooth continuous wave (shimmer / flow) -------------------------------
// The self-animated states render a flowing sine "ribbon" rather than discrete
// bars: two layered sine lines drift at different speeds for an organic feel.
// Geometry is built once at module load; CSS translates the lines and loops
// seamlessly — each <svg> is 200% wide and shifts by -50% (an integer number
// of cycles), so the wrap is invisible.
const WAVE_W = 200;
const WAVE_H = 40;

/** Build a smooth sine path across the full wave viewBox. `period` must divide
 *  100 so the -50% travel loops seamlessly. */
function sinePath(period: number, amp: number, phase: number): string {
  const mid = WAVE_H / 2;
  let d = "";
  for (let x = 0; x <= WAVE_W; x += 2) {
    const y = mid + amp * Math.sin((2 * Math.PI * x) / period + phase);
    d += `${x === 0 ? "M" : "L"}${x} ${y.toFixed(2)} `;
  }
  return d.trim();
}

const FRONT_PATH = sinePath(50, 12, 0);
// Same curve, closed down to the baseline — a soft "ribbon" body under the crest.
const FRONT_FILL = `${FRONT_PATH} L ${WAVE_W} ${WAVE_H} L 0 ${WAVE_H} Z`;
const BACK_PATH = sinePath(100, 8, 1.1);

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
 * Voice waveform with two looks:
 *
 * - **reactive** (listening): a mirrored "voice spindle" of bars driven by the
 *   live 16-band vocal spectrum, smoothed frame-to-frame.
 * - **shimmer / flow** (working / speaking): a smooth, continuously flowing
 *   sine ribbon — two sine lines drifting at different speeds — for a calm,
 *   premium motion that doesn't react to audio.
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

  // Unique bands from the centre (low freq, loud) out to one edge.
  const half = Math.max(2, Math.ceil(count / 2));
  const smoothedRef = useRef<number[]>(new Array(half).fill(0));
  if (smoothedRef.current.length !== half) {
    smoothedRef.current = new Array(half).fill(0);
  }
  // Stable, collision-free id for the ribbon's fill gradient.
  const fillId = `wfill-${useId().replace(/:/g, "")}`;

  // Self-animated states draw a smooth, flowing sine ribbon (no live audio).
  if (mode !== "reactive") {
    return (
      <div
        className={`audio-waveform ${size} ${mode} ${className}`}
        aria-hidden="true"
      >
        <div className="wave-flow-field">
          <svg
            className="wave-line back"
            viewBox={`0 0 ${WAVE_W} ${WAVE_H}`}
            preserveAspectRatio="none"
          >
            <path className="wline" d={BACK_PATH} />
          </svg>
          <svg
            className="wave-line front"
            viewBox={`0 0 ${WAVE_W} ${WAVE_H}`}
            preserveAspectRatio="none"
          >
            <defs>
              <linearGradient id={fillId} x1="0" y1="0" x2="0" y2="1">
                <stop className="wfill-top" offset="0" />
                <stop className="wfill-bottom" offset="1" />
              </linearGradient>
            </defs>
            <path className="wfill" d={FRONT_FILL} fill={`url(#${fillId})`} />
            <path className="wline" d={FRONT_PATH} />
          </svg>
        </div>
      </div>
    );
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
      className={`audio-waveform ${size} ${mode} ${idle ? "is-idle" : ""} ${className}`}
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
              style={{ height: `${pct}%` } as React.CSSProperties}
            />
          );
        })}
      </div>
    </div>
  );
};

export default AudioWaveform;
