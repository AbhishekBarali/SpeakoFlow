import { expect, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Slider } from "./Slider";

/**
 * The slider persists when a drag ends, not on every tick of it: each commit
 * is a settings write plus a full settings refetch in two windows.
 */

let renderer: ReactTestRenderer;
const nodeMock = () => ({
  addEventListener: () => {},
  removeEventListener: () => {},
});
const mount = async (props: Partial<React.ComponentProps<typeof Slider>>) => {
  await act(async () => {
    renderer = create(
      <Slider
        value={0.5}
        onChange={() => {}}
        min={0}
        max={1}
        label="Volume"
        {...props}
      />,
      { createNodeMock: nodeMock },
    );
  });
};
const input = () => renderer.root.findByType("input");
const drag = async (value: number) => {
  await act(async () => {
    input().props.onChange({ target: { value: String(value) } });
  });
};
const release = async () => {
  await act(async () => {
    input().props.onPointerUp();
  });
};

test("a drag moves the thumb but persists only on release", async () => {
  const sent: number[] = [];
  await mount({ onChange: (v: number) => sent.push(v) });
  await drag(0.6);
  await drag(0.7);
  await drag(0.8);
  expect(input().props.value).toBe(0.8);
  expect(sent).toEqual([]);
  await release();
  expect(sent).toEqual([0.8]);
  // The other release signals of the same drag do not send it again.
  await act(async () => {
    input().props.onKeyUp();
    input().props.onBlur();
  });
  expect(sent).toEqual([0.8]);
});

test("the thumb holds the committed value until the store catches up", async () => {
  let finish: () => void = () => {};
  const sent: number[] = [];
  await mount({
    onChange: (v: number) => {
      sent.push(v);
      return new Promise<void>((resolve) => {
        finish = resolve;
      });
    },
  });
  await drag(0.9);
  await release();
  // Still in flight and `value` is still the old 0.5: no snap back.
  expect(input().props.value).toBe(0.9);
  // A second drag while the first commit is in flight waits for it.
  await drag(0.2);
  await release();
  expect(sent).toEqual([0.9]);
  await act(async () => {
    finish();
  });
  expect(sent).toEqual([0.9, 0.2]);
  await act(async () => {
    renderer.update(
      <Slider value={0.2} onChange={() => {}} min={0} max={1} label="Volume" />,
    );
    finish();
  });
  expect(input().props.value).toBe(0.2);
});

test("a refused commit shows the stored value again", async () => {
  await mount({ onChange: () => Promise.reject(new Error("nope")) });
  await drag(0.1);
  await release();
  expect(input().props.value).toBe(0.5);
});

test("a release that lands where it started sends nothing", async () => {
  const sent: number[] = [];
  await mount({ onChange: (v: number) => sent.push(v) });
  await drag(0.7);
  await drag(0.5);
  await release();
  expect(sent).toEqual([]);
  expect(input().props.value).toBe(0.5);
});

test("liveCommitMs also commits during the drag, throttled", async () => {
  const sent: number[] = [];
  await mount({ onChange: (v: number) => sent.push(v), liveCommitMs: 20 });
  await drag(0.6);
  await drag(0.7);
  expect(sent).toEqual([]);
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 40));
  });
  expect(sent).toEqual([0.7]);
  await drag(0.75);
  await release();
  expect(sent).toEqual([0.7, 0.75]);
});
