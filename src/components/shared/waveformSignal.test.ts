import { describe, expect, test } from "bun:test";
import {
  MAX_HEIGHT,
  REST_HEIGHT,
  stepSpring,
  voiceEnergy,
  waveTargets,
} from "./waveformSignal";
const speech = [
  0.15, 0.28, 0.55, 0.72, 0.66, 0.48, 0.35, 0.3, 0.22, 0.18, 0.12, 0.09, 0.07,
  0.05, 0.04, 0.03,
];
describe("speech-driven waveform", () => {
  test("silence and room noise have no forced listening motion", () => {
    for (const levels of [[], Array(16).fill(0), Array(16).fill(0.05)]) {
      expect(voiceEnergy(levels)).toBe(0);
      expect(waveTargets(levels, 14)).toEqual(Array(14).fill(REST_HEIGHT));
    }
  });
  test("soft and loud speech keep the same comfortable size and contour", () => {
    const normal = waveTargets(speech, 14);
    for (const gain of [0.25, 0.5, 1.25]) {
      const scaled = waveTargets(
        speech.map((value) => value * gain),
        14,
      );
      scaled.forEach((height, index) =>
        expect(height).toBeCloseTo(normal[index], 6),
      );
      expect(Math.max(...scaled)).toBeGreaterThan(0.5);
      expect(Math.max(...scaled)).toBeLessThanOrEqual(0.72);
    }
    expect(Math.max(...waveTargets(Array(16).fill(1), 14))).toBeLessThan(
      MAX_HEIGHT,
    );
  });
  test("voice pitch does not pin the motion to one side", () => {
    const low = waveTargets([0.1, 0.8, 1, 0.6, 0.1, 0.1, 0.1, 0.1], 14, 0.45);
    const high = waveTargets([0.1, 0.1, 0.1, 0.1, 0.1, 0.6, 1, 0.8], 14, 0.45);
    expect(low).toEqual(high);
  });
  test("a crest travels along the row rather than every bar pulsing at once", () => {
    // Each bar's own height over time, so the shape is compared by when a bar
    // peaks rather than by how tall it gets — the edge taper only scales
    // amplitude, and the correlation below is normalized against that.
    const seconds = 10,
      count = 14;
    const traces = Array.from({ length: count }, (_, bar) =>
      Array.from(
        { length: seconds * 60 },
        (_, frame) => waveTargets(speech, count, frame / 60)[bar],
      ),
    );
    const standardize = (trace: number[]) => {
      const mean = trace.reduce((sum, value) => sum + value, 0) / trace.length;
      const centred = trace.map((value) => value - mean);
      const deviation = Math.sqrt(
        centred.reduce((sum, value) => sum + value * value, 0) / centred.length,
      );
      return centred.map((value) => value / deviation);
    };
    /** Frames of delay at which one bar best explains its neighbour. */
    const lag = (left: number[], right: number[]) => {
      const a = standardize(left),
        b = standardize(right);
      let best = -Infinity,
        at = 0;
      for (let shift = -12; shift <= 12; shift++) {
        let dot = 0,
          samples = 0;
        for (let i = 0; i < a.length; i++) {
          const j = i + shift;
          if (j < 0 || j >= b.length) continue;
          dot += a[i] * b[j];
          samples++;
        }
        if (dot / samples > best) {
          best = dot / samples;
          at = shift;
        }
      }
      return { at, correlation: best };
    };
    for (let bar = 0; bar + 1 < count; bar++) {
      const { at, correlation } = lag(traces[bar], traces[bar + 1]);
      // One direction, one steady speed: a crest reaches the next bar about a
      // tenth of a second later. Zero lag everywhere is the row pulsing in
      // unison, which is what this shape replaced.
      expect(at).toBeGreaterThanOrEqual(4);
      expect(at).toBeLessThanOrEqual(9);
      expect(correlation).toBeGreaterThan(0.7);
    }
  });
  test("every bar varies in size, smoothly, without collapsing or clipping", () => {
    const frames = Array.from({ length: 300 }, (_, i) =>
      waveTargets(speech, 14, i / 60),
    );
    for (let bar = 0; bar < 14; bar++) {
      const heights = frames.map((frame) => frame[bar]);
      // Visible travel of at least a couple of pixels for the tapered end bars
      // and much more in the middle, but never a bar that reads as switched off.
      expect(Math.max(...heights) - Math.min(...heights)).toBeGreaterThan(0.12);
      expect(Math.min(...heights)).toBeGreaterThan(REST_HEIGHT);
      expect(Math.max(...heights)).toBeLessThan(MAX_HEIGHT);
      // Under half a pixel of change per frame at 60 Hz: motion, not jitter.
      for (let i = 1; i < heights.length; i++)
        expect(Math.abs(heights[i] - heights[i - 1])).toBeLessThan(0.03);
    }
    // Bars differ from each other at any instant, and the row is not a mirror.
    const spread = frames.map(
      (frame) => Math.max(...frame) - Math.min(...frame),
    );
    expect(Math.min(...spread)).toBeGreaterThan(0.1);
    expect(
      Math.max(
        ...frames.flatMap((frame) =>
          frame.map((height, index) => Math.abs(height - frame[13 - index])),
        ),
      ),
    ).toBeGreaterThan(0.15);
  });
  test("invalid microphone values cannot create invalid geometry", () => {
    for (const levels of [
      [NaN, Infinity, -Infinity],
      [-8, 3, 0.5],
    ]) {
      for (const value of waveTargets(levels, 14)) {
        expect(Number.isFinite(value)).toBe(true);
        expect(value).toBeGreaterThanOrEqual(REST_HEIGHT);
        expect(value).toBeLessThanOrEqual(MAX_HEIGHT);
      }
    }
  });
  test("the damped spring responds promptly and settles without bouncing", () => {
    let spring = { position: REST_HEIGHT, velocity: 0 };
    for (let i = 0; i < 8; i++) {
      const next = stepSpring(spring, 0.6, 1 / 60);
      expect(next.position).toBeGreaterThanOrEqual(spring.position);
      expect(next.position).toBeLessThanOrEqual(0.6);
      spring = next;
    }
    expect(spring.position).toBeGreaterThan(0.55);
    for (let i = 0; i < 45; i++)
      spring = stepSpring(spring, REST_HEIGHT, 1 / 60);
    expect(spring).toEqual({ position: REST_HEIGHT, velocity: 0 });
  });
  test("response timing agrees at 30, 60, and 120 Hz", () => {
    const atRate = (hz: number) => {
      let spring = { position: REST_HEIGHT, velocity: 0 };
      for (let i = 0; i < hz / 5; i++) spring = stepSpring(spring, 0.6, 1 / hz);
      return spring.position;
    };
    expect(atRate(30)).toBeCloseTo(atRate(60), 6);
    expect(atRate(120)).toBeCloseTo(atRate(60), 6);
  });
});
