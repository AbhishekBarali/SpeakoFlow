import { expect, test } from "bun:test";

import {
  createAdaptivePoll,
  MEASURE_INTERVAL_MS,
  SETTLE_AFTER_MEASURES,
  SETTLED_INTERVAL_MS,
  unionHitRect,
  type MeasuredSurface,
  type PollClock,
} from "./hitRegion";

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
  const pill = { x: 92, y: 11, width: 155, height: 34 };
  expect(unionHitRect([surface(92, 11, 247, 45)], 1)).toEqual({
    kind: "rect",
    rect: pill,
    rects: [pill],
  });
});

test("several drawn surfaces are each reported, with the box around them", () => {
  expect(
    unionHitRect([surface(10, 10, 60, 40), surface(100, 30, 140, 90)], 1),
  ).toEqual({
    kind: "rect",
    rect: { x: 10, y: 10, width: 130, height: 80 },
    rects: [
      { x: 10, y: 10, width: 50, height: 30 },
      { x: 100, y: 30, width: 40, height: 60 },
    ],
  });
});

test("a wide call bubble does not make the corners beside the bar tangible", () => {
  // A 440x200 call window: "Searching the web · …" at 400px wide above a 170px
  // bar. The box around the two takes in the empty space either side of the bar,
  // which is where the user's own app is — so each surface is reported on its
  // own, and that space is in none of them.
  const region = unionHitRect(
    [surface(20, 100, 420, 136), surface(135, 146, 305, 184)],
    1,
  );
  expect(region.kind).toBe("rect");
  if (region.kind !== "rect") return;
  const inAny = (x: number, y: number) =>
    region.rects.some(
      (r) => x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height,
    );
  expect(inAny(220, 165)).toBe(true); // on the bar
  expect(inAny(220, 118)).toBe(true); // on the bubble
  expect(inAny(40, 165)).toBe(false); // beside the bar, under the bubble
  expect(inAny(400, 165)).toBe(false);
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
  const scaled = { x: 138, y: 16.5, width: 232.5, height: 51 };
  expect(unionHitRect([surface(92, 11, 247, 45)], 1.5)).toEqual({
    kind: "rect",
    rect: scaled,
    rects: [scaled],
  });
});

test("a nonsense device pixel ratio does not collapse the rect", () => {
  // A zero or negative ratio would multiply the drawn area to nothing, which reads
  // as "pass everything through" — i.e. a visible panel nobody can click.
  const pill = { x: 92, y: 11, width: 155, height: 34 };
  expect(unionHitRect([surface(92, 11, 247, 45)], 0)).toEqual({
    kind: "rect",
    rect: pill,
    rects: [pill],
  });
});

/** A clock that only moves when the test says so. */
function manualClock() {
  let pending: { run: () => void; ms: number; id: number } | null = null;
  let nextId = 0;
  const clock: PollClock = {
    schedule: (run, ms) => {
      pending = { run, ms, id: ++nextId };
      return pending.id;
    },
    cancel: (handle) => {
      if (pending?.id === handle) pending = null;
    },
  };
  return {
    clock,
    pendingMs: () => pending?.ms ?? null,
    fire: () => {
      const due = pending;
      pending = null;
      due?.run();
    },
  };
}

test("a still surface settles to the slow pace, a change snaps it back", () => {
  const time = manualClock();
  let changed = true;
  let measures = 0;
  const poll = createAdaptivePoll({
    tick: () => {
      measures++;
      return changed;
    },
    clock: time.clock,
  });
  poll.start();
  expect(measures).toBe(1);
  expect(time.pendingMs()).toBe(MEASURE_INTERVAL_MS);

  // Moving keeps it fast however long it goes on.
  for (let i = 0; i < 30; i++) time.fire();
  expect(time.pendingMs()).toBe(MEASURE_INTERVAL_MS);

  // Holding still for the settle count slows it down.
  changed = false;
  for (let i = 0; i < SETTLE_AFTER_MEASURES; i++) time.fire();
  expect(time.pendingMs()).toBe(SETTLED_INTERVAL_MS);

  // A measured change at the slow pace is enough to speed it up again.
  changed = true;
  time.fire();
  expect(time.pendingMs()).toBe(MEASURE_INTERVAL_MS);
});

test("a wake while settled measures at once instead of waiting out the slow tick", () => {
  const time = manualClock();
  let measures = 0;
  const poll = createAdaptivePoll({
    tick: () => {
      measures++;
      return false;
    },
    clock: time.clock,
  });
  poll.start();
  for (let i = 0; i < SETTLE_AFTER_MEASURES; i++) time.fire();
  expect(time.pendingMs()).toBe(SETTLED_INTERVAL_MS);

  const before = measures;
  poll.wake();
  expect(measures).toBe(before + 1);
  expect(time.pendingMs()).toBe(MEASURE_INTERVAL_MS);

  // At the fast pace a wake only restarts the settle count: no extra measure,
  // and the pending tick is still the next one.
  poll.wake();
  expect(measures).toBe(before + 1);
  expect(time.pendingMs()).toBe(MEASURE_INTERVAL_MS);
  for (let i = 0; i < SETTLE_AFTER_MEASURES - 1; i++) time.fire();
  expect(time.pendingMs()).toBe(MEASURE_INTERVAL_MS);
});

test("a stopped poll neither measures nor schedules again", () => {
  const time = manualClock();
  let measures = 0;
  const poll = createAdaptivePoll({
    tick: () => {
      measures++;
      return false;
    },
    clock: time.clock,
  });
  poll.start();
  poll.stop();
  expect(time.pendingMs()).toBeNull();
  poll.wake();
  time.fire();
  expect(measures).toBe(1);
});
