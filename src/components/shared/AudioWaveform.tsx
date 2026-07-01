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

// Reactive-mode shaping (live mic). The vocal spectrum coming from the backend
// tends to sit low, so instead of mapping it straight onto 0..1 we lift it and
// hold it inside a pleasing visual band:
//   - REACTIVE_GAIN lifts the generally-low incoming signal so ordinary speech
//     actually fills the wave (fixes "too low").
//   - REACTIVE_CEIL caps the amplitude so loud peaks can't slam the bars to
//     full height and read as a jagged, bizarre spike (the upper limit).
//   - ACTIVE_FLOOR keeps a baseline height while the mic is live so the wave
//     always reads as a continuous flow instead of collapsing to flat dots
//     between syllables (the lower limit).
const REACTIVE_GAIN = 1.7;
const REACTIVE_CEIL = 0.9;
const ACTIVE_FLOOR = 0.34;

// Frame-to-frame easing for the live wave, applied per spectrum update
// (~20-30/sec). Lower = smoother and slower. The release (falling) rate is
// deliberately much slower than the attack (rising) rate so the bars climb
// gently toward louder audio and then ease back down like a slow tide — they
// never snap up or drop out from under the eye between syllables. This is what
// turns a twitchy, distracting meter into a soothing flow.
const WAVE_ATTACK = 0.28;
const WAVE_RELEASE = 0.1;

// Envelope floor shared by every mode. Rather than a tall "spindle" that tapers
// to empty dots at the rim, the bars follow a gentle arch whose sides keep this
// share of the centre's height — so the wave (live mic, thinking, or speaking)
// reads as an even flow across the whole chip instead of a spike in the middle
// with hollow sides.
const ENV_FLOOR = 0.68;

// Resting silhouette height for the self-animated states, as a share of the
// reactive range — `flow` (speaking) stands taller and livelier than the calm
// `shimmer` (thinking). CSS then pulses each bar around this baseline.
const SELF_SILHOUETTE: Record<Exclude<WaveMode, "reactive">, number> = {
  shimmer: 0.5,
  flow: 0.82,
};

/** Envelope across the bars: 1 at the centre line, easing to ENV_FLOOR at the
 *  rim (a raised cosine). A gentle arch that keeps the sides full so the wave
 *  reads as an even flow rather than a spike in the middle with empty dots. */
function envelope(dist: number): number {
  const arch = 0.5 + 0.5 * Math.cos(Math.PI * Math.min(1, dist));
  return ENV_FLOOR + (1 - ENV_FLOOR) * arch;
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
 * arch of rounded-cap bars that rise from a centre line, with the sides kept
 * full (see `envelope`) so it reads as an even flow rather than a spike.
 *
 * - **reactive** (listening): bar heights are driven by the live 16-band vocal
 *   spectrum, smoothed frame-to-frame.
 * - **shimmer / flow** (working / speaking): the bars hold a calm arch
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
    // Asymmetric attack/release easing: move toward a louder target at
    // WAVE_ATTACK, but fall back at the slower WAVE_RELEASE, so the wave settles
    // gently instead of tracking every jump. Both rates are gentle enough that
    // the motion reads as a flowing tide rather than a reactive meter.
    display = smoothedRef.current.map((prev, i) => {
      const target = profile[i] ?? 0;
      const k = target > prev ? WAVE_ATTACK : WAVE_RELEASE;
      return prev + (target - prev) * k;
    });
    smoothedRef.current = display;
    const energy = display.reduce((m, v) => Math.max(m, v), 0);
    // A low trip point so brief gaps between words don't drop the wave into the
    // idle "breathe" state; the active floor keeps it flowing while speaking.
    idle = !active || energy < 0.03;
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
          // Flatter, floored arch (see `envelope`): the centre leads slightly
          // but the sides stay full, so every mode flows evenly across the chip
          // rather than spiking in the middle with empty sides.
          const env = envelope(dist);

          if (isReactive) {
            const band = display[Math.round(dist * (half - 1))] ?? 0;
            // Lift the low-sitting signal with the sub-1 curve + gain, then cap
            // it so loud peaks stay smooth rather than spiking to the rim.
            const shaped = Math.pow(Math.min(1, Math.max(0, band)), 0.6);
            const signal = Math.min(REACTIVE_CEIL, shaped * REACTIVE_GAIN);
            // While the mic is live, blend in a baseline so the wave keeps
            // flowing between syllables.
            const base = idle ? 0 : ACTIVE_FLOOR + (1 - ACTIVE_FLOOR) * signal;
            const amp = base * env;
            const pct = MIN_PCT + amp * (MAX_PCT - MIN_PCT);
            return (
              <span
                key={i}
                className="wave-bar"
                style={{ height: `${pct}%` } as React.CSSProperties}
              />
            );
          }

          // Self-animated: a static arch silhouette that CSS pulses around.
          // `--dist` phases the centre-out ripple; the height is the resting
          // shape (flattened envelope) scaled by the per-mode silhouette weight.
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
