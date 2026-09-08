import { expect, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import OverlayProgress from "./OverlayProgress";

const render = (props: {
  label: string;
  completed: boolean;
  active: boolean;
}) => {
  let renderer!: ReactTestRenderer;
  act(() => {
    renderer = create(<OverlayProgress {...props} />);
  });
  const bar = () => renderer.root.findByProps({ role: "progressbar" }).props;
  return {
    bar,
    update: (next: Partial<typeof props>) =>
      act(() => {
        renderer.update(<OverlayProgress {...props} {...next} />);
      }),
  };
};

test("waiting reports busy without inventing a percentage", () => {
  const { bar } = render({
    label: "Cleaning up…",
    completed: false,
    active: true,
  });
  expect(bar()["aria-valuenow"]).toBeUndefined();
  expect(bar()["aria-label"]).toBe("Cleaning up…");
  expect(bar().className).toBe("overlay-progress");
});

test("a hidden overlay parks the sweep instead of animating unseen", () => {
  // The pause is entirely class-driven now, and an indeterminate sweep has no
  // position to restore — so this class is the only thing keeping a hidden
  // window from running a compositor animation for the whole wait.
  const { bar, update } = render({
    label: "Transcribing…",
    completed: false,
    active: false,
  });
  expect(bar().className).toContain("is-paused");
  update({ active: true });
  expect(bar().className).not.toContain("is-paused");
});

test("only a real result completes the bar, and it stops sweeping when it does", () => {
  const { bar, update } = render({
    label: "Transcribing…",
    completed: false,
    active: true,
  });
  update({ completed: true });
  expect(bar()["aria-valuenow"]).toBe(100);
  expect(bar().className).toContain("is-complete");
});
