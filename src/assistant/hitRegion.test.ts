import { expect, test } from "bun:test";

import { unionHitRect, type MeasuredSurface } from "./hitRegion";

/**
 * What the assistant panel tells Rust it is drawing.
 *
 * This is the measurement behind "nothing on my screen is clickable while the
 * assistant is open". The panel window is always bigger than its content — the ask
 * pill renders at roughly 155x34 inside a 340x56 window — and the surplus used to
 * be invisible but tangible, eating clicks and answering right-clicks with
 * WebView2's own Copy / Print / Inspect menu. Everything outside the rect measured
 * here is handed back to whatever is underneath, so getting it wrong is the
 * difference between a usable desktop and a dead one.
 */

const surface = (
  left: number,
  top: number,
  right: number,
  bottom: number,
  drawn = true,
): MeasuredSurface => ({ drawn, left, top, right, bottom });

test("the pill's own box is the hit rect, not the window it floats in", () => {
  // The pill as it actually renders: centred in a 340x56 frame.
  expect(unionHitRect([surface(92, 11, 247, 45)], 1)).toEqual({
    kind: "rect",
    rect: { x: 92, y: 11, width: 155, height: 34 },
  });
});

test("several drawn surfaces are covered by one rect", () => {
  expect(
    unionHitRect([surface(10, 10, 60, 40), surface(100, 30, 140, 90)], 1),
  ).toEqual({ kind: "rect", rect: { x: 10, y: 10, width: 130, height: 80 } });
});

test("surfaces that are all faded out pass everything through", () => {
  // Mid cross-fade: the pill is leaving (pointer-events: none) and the card has not
  // arrived. Nothing is on screen, so nothing should be catching clicks.
  expect(
    unionHitRect(
      [surface(92, 11, 247, 45, false), surface(0, 0, 400, 300, false)],
      1,
    ),
  ).toEqual({ kind: "none" });
});

test("a form with no measurable surface stays fully tangible", () => {
  // The voice conversation view and the full chat panel have none of the listed
  // surfaces in them. Reporting an empty rect here — which an earlier version of
  // this did — makes the whole window pass-through, so a live call has no reachable
  // Mute or End button. `unknown` is a different answer from `none` for exactly this
  // reason, and the distinction is not cosmetic.
  expect(unionHitRect([], 1)).toEqual({ kind: "unknown" });
});

test("a zero-area surface counts as not drawn rather than as a point", () => {
  // A collapsed element would otherwise leave a one-pixel island of live window.
  expect(unionHitRect([surface(120, 30, 120, 30)], 1)).toEqual({
    kind: "none",
  });
});

test("the rect is reported in physical pixels", () => {
  // The window origin Rust compares against is physical, so a scaled display has to
  // be converted here — this side is the only one that knows the ratio.
  expect(unionHitRect([surface(92, 11, 247, 45)], 1.5)).toEqual({
    kind: "rect",
    rect: { x: 138, y: 16.5, width: 232.5, height: 51 },
  });
});

test("a nonsense device pixel ratio does not collapse the rect", () => {
  // A zero or negative ratio would multiply the drawn area to nothing, which reads
  // as "pass everything through" — i.e. a visible panel nobody can click.
  expect(unionHitRect([surface(92, 11, 247, 45)], 0)).toEqual({
    kind: "rect",
    rect: { x: 92, y: 11, width: 155, height: 34 },
  });
});
