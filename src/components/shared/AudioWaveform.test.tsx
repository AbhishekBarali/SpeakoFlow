import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import AudioWaveform from "./AudioWaveform";
let renderer: ReactTestRenderer;
let now = 1000,
  serial = 0;
const frames = new Map<number, FrameRequestCallback>();
const originalRequest = globalThis.requestAnimationFrame;
const originalCancel = globalThis.cancelAnimationFrame;
let time: ReturnType<typeof spyOn>;
beforeEach(() => {
  now = 1000;
  frames.clear();
  time = spyOn(performance, "now").mockImplementation(() => now);
  globalThis.requestAnimationFrame = (callback) => {
    frames.set(++serial, callback);
    return serial;
  };
  globalThis.cancelAnimationFrame = (id) => {
    frames.delete(id);
  };
  act(() => {
    renderer = create(<AudioWaveform levels={[]} />, {
      createNodeMock: () => ({ closest: () => null }),
    });
  });
});
afterEach(() => {
  act(() => renderer.unmount());
  time.mockRestore();
  globalThis.requestAnimationFrame = originalRequest;
  globalThis.cancelAnimationFrame = originalCancel;
});
const tick = (count = 1) => {
  for (let i = 0; i < count; i++) {
    now += 1000 / 60;
    const pending = [...frames.values()];
    frames.clear();
    act(() => {
      pending.forEach((callback) => callback(now));
    });
  }
};
const levels = [0.1, 0.3, 0.8, 0.6, 0.2, 0.1];
const show = (values: number[], active = true) =>
  act(() => renderer.update(<AudioWaveform levels={values} active={active} />));
const heights = () =>
  renderer.root
    .findAllByType("line")
    .map((line) => line.props.y2 - line.props.y1);
test("an open, silent microphone does not schedule animation frames", () => {
  show(Array(16).fill(0.05));
  tick(60);
  expect(frames.size).toBe(0);
  expect(heights().every((height) => height === 2)).toBe(true);
});
test("speech wakes the spring; silence releases it and stops the clock", () => {
  show(levels);
  tick(7);
  expect(Math.max(...heights())).toBeGreaterThan(6);
  show([]);
  tick(45);
  expect(frames.size).toBe(0);
  expect(heights().every((height) => height === 2)).toBe(true);
  show(levels);
  expect(frames.size).toBe(1);
  show([], false);
  expect(frames.size).toBe(0);
});
test("a microphone that stops publishing cannot leave the last loud frame active", () => {
  show(levels);
  tick(60);
  expect(frames.size).toBe(0);
  expect(heights().every((height) => height === 2)).toBe(true);
});
test("steady speech keeps the bars moving vertically in place", () => {
  for (let i = 0; i < 40; i++) {
    show([...levels]);
    tick(2);
  }
  const steady = heights();
  for (let i = 0; i < 20; i++) {
    show([...levels]);
    tick(2);
  }
  expect(heights()).not.toEqual(steady);
  expect(heights().every((height) => height > 4 && height < 14)).toBe(true);
});

test("brief syllable gaps hold the wave, but continuing silence parks it", () => {
  for (let i = 0; i < 25; i++) {
    show([...levels]);
    tick(2);
  }
  show([]);
  tick(4);
  expect(Math.max(...heights())).toBeGreaterThan(7.5);
  tick(55);
  expect(frames.size).toBe(0);
  for (let i = 0; i < 20; i++) {
    show(Array(16).fill(0));
    expect(frames.size).toBe(0);
    tick(2);
  }
});

test("moving closer to the microphone does not inflate the speaking wave", () => {
  let soft: number[] = [];
  for (const gain of [0.25, 1]) {
    show([], false);
    for (let i = 0; i < 30; i++) {
      show(levels.map((value) => value * gain));
      tick(2);
    }
    if (gain === 0.25) soft = heights();
    else
      heights().forEach((height, index) =>
        expect(height).toBeCloseTo(soft[index], 6),
      );
  }
});
