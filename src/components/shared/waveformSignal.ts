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

/** The shape of the speaking wave, as travelling components rather than a fixed
 * per-bar table. Each one contributes `cycles` crests across the row and moves
 * along it once per `period`, so a crest is visibly handed from one bar to the
 * next — that phase gradient is the whole difference between a wave and a row of
 * bars pulsing in unison, which is what a single shared phase produced before.
 *
 * The periods are deliberately incommensurate and the third component travels
 * the other way. Two same-direction waves alone read as a conveyor belt on a
 * fixed beat; adding a slow counter-swell means the crests never line up the
 * same way twice, so the silhouette keeps changing size without anything
 * jumping. */
const COMPONENTS = [
  // Primary crest: ~1.5 crests in view, so a crest is always somewhere on the
  // row, sweeping across it a little over once per second.
  { cycles: 1.5, period: 0.9, direction: 1, weight: 0.5, offset: 0 },
  // Shorter ripple riding along with it, at a slower tempo.
  { cycles: 2.6, period: 1.37, direction: 1, weight: 0.33, offset: 1.7 },
  // Long counter-travelling swell; breaks up any repeating march.
  { cycles: 0.85, period: 1.13, direction: -1, weight: 0.24, offset: 3.1 },
];
const TOTAL_WEIGHT = COMPONENTS.reduce((sum, c) => sum + c.weight, 0);
/** Bar height at a trough, and how much a full crest adds. The floor stays well
 * clear of REST_HEIGHT so no bar reads as dead while speech is coming in. */
const WAVE_FLOOR = 0.19;
const WAVE_SPAN = 0.53;
/** A slow breath over the whole row, so successive crests are not all the same
 * height. Shallow on purpose: this is the "varies a little" part, not a pump. */
const SWELL_PERIOD = 2.9;
const SWELL_DEPTH = 0.14;

/** Shortens the outermost bars so the row has a rounded silhouette and crests
 * appear to enter and leave rather than being clipped at the ends. */
const edgeTaper = (u: number) =>
  0.68 + 0.32 * Math.pow(Math.sin(Math.PI * u), 0.75);

/** Speech-gated activity, as a wave that travels along the row at a steady
 * tempo and comfortable size. This is an activity indicator rather than a
 * literal spectrum: `seconds` drives the motion and the microphone only decides
 * whether it runs, so leaning into the mic changes nothing about how big it is. */
export function speechWave(count: number, seconds: number): number[] {
  const time = Number.isFinite(seconds) ? seconds : 0;
  const swell =
    1 - (SWELL_DEPTH * (1 - Math.sin((2 * Math.PI * time) / SWELL_PERIOD))) / 2;
  return Array.from({ length: count }, (_, index) => {
    const u = count === 1 ? 0.5 : index / (count - 1);
    let sum = 0;
    for (const { cycles, period, direction, weight, offset } of COMPONENTS)
      sum +=
        weight *
        Math.sin(
          2 * Math.PI * (cycles * u - (direction * time) / period) + offset,
        );
    const crest = 0.5 + sum / (2 * TOTAL_WEIGHT);
    return edgeTaper(u) * (WAVE_FLOOR + WAVE_SPAN * swell * crest);
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
