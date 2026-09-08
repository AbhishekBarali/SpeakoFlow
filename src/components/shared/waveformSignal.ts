export const REST_HEIGHT = 0.1;
export const MAX_HEIGHT = 0.8;
export const AUDIO_STALE_MS = 180;
export const SPEECH_RELEASE_MS = 120;
const NOISE_GATE = 0.08;
const clamp01 = (value: number) =>
  Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0;

/** Detect input above the backend's normalized room noise. Detection is kept
 * separate from display size, so moving nearer the mic does not enlarge it. */
export function voiceEnergy(levels: readonly number[]): number {
  if (!levels.length) return 0;
  let sum = 0,
    peak = 0;
  for (const raw of levels) {
    const value = clamp01(raw);
    sum += value;
    peak = Math.max(peak, value);
  }
  const signal = (sum / levels.length) * 0.34 + peak * 0.66;
  return Math.pow(Math.max(0, (signal - NOISE_GATE) / (1 - NOISE_GATE)), 0.95);
}

// Rounded vertical bars with a balanced, fixed profile. The small paired offsets
// soften the up/down rhythm; there is no phase progression across the row.
const BAR_PROFILE = [
  0.64, 0.84, 1, 0.78, 0.9, 0.69, 0.94, 0.94, 0.69, 0.9, 0.78, 1, 0.84, 0.64,
];
const BAR_PHASE = [
  0.12, -0.08, 0.04, 0.16, -0.12, 0.08, 0, 0, 0.08, -0.12, 0.16, 0.04, -0.08,
  0.12,
];

/** Speech-gated vertical activity, with a steady size and tempo. This is an
 * activity indicator rather than a literal spectrum or scrolling waveform. */
export function speechWave(count: number, seconds: number): number[] {
  const phase = ((Number.isFinite(seconds) ? seconds : 0) * 2 * Math.PI) / 0.82;
  return Array.from({ length: count }, (_, index) => {
    const position = count === 1 ? 6.5 : (index * 13) / (count - 1);
    const left = Math.floor(position),
      fraction = position - left;
    const sample = (values: number[]) =>
      values[left] * (1 - fraction) + values[Math.min(13, left + 1)] * fraction;
    const lift = (1 + Math.sin(phase + sample(BAR_PHASE))) / 2;
    return 0.24 + sample(BAR_PROFILE) * (0.15 + 0.31 * lift);
  });
}

export function waveTargets(
  levels: readonly number[],
  count: number,
  seconds = 0,
): number[] {
  return voiceEnergy(levels)
    ? speechWave(count, seconds)
    : Array(count).fill(REST_HEIGHT);
}

export interface Spring {
  position: number;
  velocity: number;
}
/** Exact critically damped response for one frame. No bouncing or perpetual
 * motion; using elapsed time gives the same response on 60 and 120 Hz screens. */
export function stepSpring(
  spring: Spring,
  target: number,
  seconds: number,
): Spring {
  const dt = Math.min(0.05, Math.max(0, seconds));
  const omega = target > spring.position ? 38 : 22;
  const offset = spring.position - target;
  const momentum = spring.velocity + omega * offset;
  const decay = Math.exp(-omega * dt);
  const position = target + (offset + momentum * dt) * decay;
  const velocity = (spring.velocity - omega * momentum * dt) * decay;
  if (Math.abs(position - target) < 0.0005 && Math.abs(velocity) < 0.005)
    return { position: target, velocity: 0 };
  return {
    position: Math.max(REST_HEIGHT, Math.min(MAX_HEIGHT, position)),
    velocity,
  };
}
