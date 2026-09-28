import { expect, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import OverlayWorking from "./OverlayWorking";

const render = (props: { label: string; active: boolean }) => {
  let renderer!: ReactTestRenderer;
  act(() => {
    renderer = create(<OverlayWorking {...props} />);
  });
  const indicator = () =>
    renderer.root.findByProps({ role: "progressbar" }).props;
  return {
    renderer,
    indicator,
    update: (next: Partial<typeof props>) =>
      act(() => {
        renderer.update(<OverlayWorking {...props} {...next} />);
      }),
  };
};

test("waiting reports busy without inventing a percentage", () => {
  const { indicator, renderer } = render({
    label: "Cleaning up…",
    active: true,
  });
  expect(indicator()["aria-valuenow"]).toBeUndefined();
  expect(indicator()["aria-label"]).toBe("Cleaning up…");
  expect(indicator().className).toBe("overlay-working");
  // Three dots, no sweeping bar.
  expect(
    renderer.root.findAllByProps({ className: "overlay-working-dot" }),
  ).toHaveLength(3);
  expect(
    renderer.root.findAllByProps({ className: "progress-sheen" }),
  ).toHaveLength(0);
});

test("a hidden overlay parks the dots instead of animating unseen", () => {
  const { indicator, update } = render({
    label: "Transcribing…",
    active: false,
  });
  expect(indicator().className).toContain("is-paused");
  update({ active: true });
  expect(indicator().className).not.toContain("is-paused");
});
