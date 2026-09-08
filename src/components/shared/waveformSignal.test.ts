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
      expect(Math.max(...scaled)).toBeLessThanOrEqual(0.7);
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
  test("bars rise and fall in place with no lateral drift or front bias", () => {
    const frames = Array.from({ length: 90 }, (_, i) =>
      waveTargets(speech, 14, i / 60),
    );
    for (let bar = 0; bar < 14; bar++) {
      const heights = frames.map((frame) => frame[bar]);
      expect(Math.max(...heights) - Math.min(...heights)).toBeGreaterThan(0.18);
      frames.forEach((frame) =>
        expect(frame[bar]).toBeCloseTo(frame[13 - bar], 8),
      );
      for (let i = 1; i < heights.length; i++)
        expect(Math.abs(heights[i] - heights[i - 1])).toBeLessThan(0.022);
    }
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
