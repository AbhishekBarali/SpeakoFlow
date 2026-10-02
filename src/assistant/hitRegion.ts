/**
 * What the assistant panel is actually drawing, reported to Rust so the rest of
 * the window can pass clicks through to whatever is underneath it.
 *
 * The panel window is deliberately larger than its content: the quick ask is a
 * fixed frame as large as the biggest card it may show, with the pill or the card
 * drawn inside it, which is what lets the surface grow from a pill into a card
 * without a window resize. Invisible is not the same as intangible, though — that
 * surplus used to sit in front of the desktop and swallow every click and
 * right-click landing on it, which made the screen feel dead whenever the
 * assistant was up.
 *
 * Only the webview can say where the drawn edge is, so it measures and Rust
 * decides: see the cursor pass-through section in `assistant.rs`.
 */

import { emit } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { suppressBrowserContextMenu } from "@/lib/contextMenu";

/**
 * The surfaces that are genuinely visible and clickable in each form the window
 * takes. Everything else in the tree is a transparent frame or a centring box.
 *
 * The quick ask is one element, `.qa-surface`, inside a fixed frame much larger
 * than it (the frame is the largest the card may grow to, so the window never
 * resizes while an ask runs). The live call opts in with `[data-hit-surface]` on
 * its bar, its status bubble and (expanded) its whole panel.
 *
 * **Every visible surface must be listed, and an omission is not a partial
 * failure.** Nothing matched means `unknown`, which keeps the whole window
 * tangible — the frame then eats clicks on the desktop around the surface. A
 * listed surface that measures as not drawn means `none`, which makes the whole
 * window pass-through. `[data-hit-surface]` is offered alongside the class list
 * so a new surface can opt in at its own element.
 */
export const HIT_SURFACE_SELECTORS = [
  "[data-hit-surface]",
  ".qa-surface",
] as const;

const HIT_SURFACES = HIT_SURFACE_SELECTORS.join(", ");

/**
 * How often the drawn rect is re-measured while anything is moving.
 *
 * The quick ask's surface animates its width and grows as an answer streams in,
 * so this has to track a transition rather than just react to a render.
 * Measuring is a `getBoundingClientRect` and a `getComputedStyle` per surface,
 * and after any DOM or style change that read forces a synchronous layout, so
 * the poll itself is the cost even though the report is change-gated.
 */
export const MEASURE_INTERVAL_MS = 50;

/**
 * The poll's pace once the surface has held still for `SETTLE_AFTER_MEASURES`
 * measurements in a row: a call can keep the panel up for an hour, and 20
 * forced layouts a second for all of it buys nothing while nothing moves.
 * Anything that could move a surface (a DOM or attribute change, a resize, a
 * transition or animation starting or ending, the pointer) snaps it straight
 * back to `MEASURE_INTERVAL_MS` and measures at once, so the slow pace only
 * covers a change that announces itself in none of those ways.
 */
export const SETTLED_INTERVAL_MS = 250;
export const SETTLE_AFTER_MEASURES = 10;

/** Physical pixels of movement worth telling Rust about. */
const CHANGE_EPSILON = 2;

export type HitRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

/** The bit of a surface this module needs: where it is, and whether it counts. */
export type MeasuredSurface = {
  drawn: boolean;
  left: number;
  top: number;
  right: number;
  bottom: number;
};

/**
 * What a measurement can conclude.
 *
 * The three cases are genuinely different and collapsing any two of them breaks
 * something. `unknown` is a form with none of these surfaces in it at all — the
 * voice conversation view, the full chat panel — and it must stay fully tangible,
 * because its Mute and End buttons are the entire point of it. `none` is a form
 * that *has* them and has faded them all out mid-cross-fade, which must pass
 * everything through or it leaves a live island of invisible window behind. Only
 * `rect` is a measurement.
 *
 * An earlier version of this had two cases and sent an empty rect for both, which
 * would have made a live call impossible to hang up.
 *
 * `rects` is every drawn surface on its own, and it is what Rust hit-tests; `rect`
 * is their bounding box, kept for logging and as a fallback. Testing against the
 * bounding box alone was a bug of its own: a call's status bubble ("Searching the
 * web · …") is far wider than the bar under it, so the box around the two took in
 * the empty corners either side of the bar, and the app underneath stopped
 * responding to the mouse there for as long as the bubble was up.
 */
export type HitRegion =
  | { kind: "unknown" }
  | { kind: "none" }
  | { kind: "rect"; rect: HitRect; rects: HitRect[] };

/**
 * Every drawn surface, in PHYSICAL pixels, plus the box around them.
 *
 * Pure and separate from the DOM walk below, because this is the arithmetic that
 * decides whether the user's desktop is reachable, and it should be answerable in
 * a test rather than by clicking around a screen.
 *
 * Physical rather than CSS pixels because the conversion needs `devicePixelRatio`
 * and only this side knows it — doing it here means the two sides of the boundary
 * never have to agree on a scale factor, which is exactly the kind of agreement
 * that breaks on a mixed-DPI desk.
 */
export function unionHitRect(
  surfaces: readonly MeasuredSurface[],
  dpr: number,
): HitRegion {
  if (surfaces.length === 0) return { kind: "unknown" };

  const scale = dpr > 0 ? dpr : 1;
  const rects: HitRect[] = [];
  let left = Infinity;
  let top = Infinity;
  let right = -Infinity;
  let bottom = -Infinity;

  for (const surface of surfaces) {
    if (!surface.drawn) continue;
    if (surface.right <= surface.left || surface.bottom <= surface.top)
      continue;
    rects.push({
      x: surface.left * scale,
      y: surface.top * scale,
      width: (surface.right - surface.left) * scale,
      height: (surface.bottom - surface.top) * scale,
    });
    left = Math.min(left, surface.left);
    top = Math.min(top, surface.top);
    right = Math.max(right, surface.right);
    bottom = Math.max(bottom, surface.bottom);
  }

  if (rects.length === 0) return { kind: "none" };
  return {
    kind: "rect",
    rect: {
      x: left * scale,
      y: top * scale,
      width: (right - left) * scale,
      height: (bottom - top) * scale,
    },
    rects,
  };
}

/**
 * Collect the drawn surfaces from the live document.
 *
 * `pointer-events` is the "is it drawn" test: it is inherited, so anything the
 * stylesheet has switched off (a surface on its way out, the call's panels while
 * they fold) is correctly reported as not drawn, using the same declaration that
 * stops it being clicked — one source of truth instead of a second opacity rule
 * to keep in sync.
 */
function surfaceNodes(root: ParentNode): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(HIT_SURFACES));
}

function measureSurfaces(nodes: readonly HTMLElement[]): MeasuredSurface[] {
  return nodes.map((node) => {
    const style = window.getComputedStyle(node);
    const rect = node.getBoundingClientRect();
    return {
      drawn: style.pointerEvents !== "none" && style.visibility !== "hidden",
      left: rect.left,
      top: rect.top,
      right: rect.right,
      bottom: rect.bottom,
    };
  });
}

function regionOf(nodes: readonly HTMLElement[]): HitRegion {
  return unionHitRect(measureSurfaces(nodes), window.devicePixelRatio || 1);
}

/** The union of everything the panel is drawing right now. */
export function measureHitRegion(root: ParentNode = document): HitRegion {
  return regionOf(surfaceNodes(root));
}

/** A cancellable one-shot timer, injectable so the pacing can be tested. */
export type PollClock = {
  schedule: (run: () => void, ms: number) => unknown;
  cancel: (handle: unknown) => void;
};

const WINDOW_CLOCK: PollClock = {
  schedule: (run, ms) => window.setTimeout(run, ms),
  cancel: (handle) => window.clearTimeout(handle as number),
};

/**
 * A poll that runs at `fastMs` and drops to `slowMs` once `tick` has reported
 * no change `settleAfter` times in a row.
 *
 * `wake()` is the "something may have moved" signal. At the fast pace it only
 * restarts the settle count (the next tick is at most `fastMs` away, exactly as
 * with a plain interval). At the slow pace it measures immediately and resumes
 * the fast pace, so a settled poll never adds lag to a change it was told about.
 */
export function createAdaptivePoll(options: {
  tick: () => boolean;
  fastMs?: number;
  slowMs?: number;
  settleAfter?: number;
  clock?: PollClock;
}) {
  const {
    tick,
    fastMs = MEASURE_INTERVAL_MS,
    slowMs = SETTLED_INTERVAL_MS,
    settleAfter = SETTLE_AFTER_MEASURES,
    clock = WINDOW_CLOCK,
  } = options;
  let unchanged = 0;
  let handle: unknown = null;
  let stopped = false;

  const settled = () => unchanged >= settleAfter;
  const run = () => {
    handle = null;
    if (stopped) return;
    unchanged = tick() ? 0 : unchanged + 1;
    if (stopped) return;
    handle = clock.schedule(run, settled() ? slowMs : fastMs);
  };

  return {
    /** Measure now and start polling. */
    start: run,
    wake() {
      if (stopped) return;
      const wasSettled = settled();
      unchanged = 0;
      if (!wasSettled) return;
      if (handle !== null) clock.cancel(handle);
      run();
    },
    stop() {
      stopped = true;
      if (handle !== null) clock.cancel(handle);
      handle = null;
    },
    /** The delay before the next measurement, for tests. */
    get intervalMs() {
      return settled() ? slowMs : fastMs;
    },
  };
}

/** Events that can start moving a surface without touching the DOM. */
const MOTION_EVENTS = [
  "transitionrun",
  "transitionstart",
  "transitionend",
  "transitioncancel",
  "animationstart",
  "animationend",
  "animationcancel",
] as const;

/** Pointer events that can change a surface through `:hover` / `:active`. */
const POINTER_WAKE_EVENTS = ["pointerover", "pointerout"] as const;

/** The payload a region is reported as. `tangible` is the "do not restrict" case. */
function payloadFor(region: HitRegion) {
  switch (region.kind) {
    case "rect":
      // The box is sent alongside the list so the payload still reads as one
      // rect to anything that only understands that shape.
      return { ...region.rect, rects: region.rects };
    case "none":
      return { x: 0, y: 0, width: 0, height: 0, rects: [] };
    case "unknown":
      return { tangible: true };
  }
}

const sameRect = (a: HitRect, b: HitRect) =>
  Math.abs(a.x - b.x) <= CHANGE_EPSILON &&
  Math.abs(a.y - b.y) <= CHANGE_EPSILON &&
  Math.abs(a.width - b.width) <= CHANGE_EPSILON &&
  Math.abs(a.height - b.height) <= CHANGE_EPSILON;

function sameRegion(a: HitRegion | null, b: HitRegion): boolean {
  if (a === null || a.kind !== b.kind) return false;
  if (a.kind !== "rect" || b.kind !== "rect") return true;
  return (
    a.rects.length === b.rects.length &&
    a.rects.every((rect, i) => sameRect(rect, b.rects[i]))
  );
}

/**
 * Report the drawn rect while the panel is on screen, and hold the window
 * tangible for the length of a drag.
 *
 * The hold is not optional. Dragging the pill hands the move to the OS, which
 * keeps the pointer captured while it travels far outside the drawn rect — so
 * without this the window would go pass-through mid-drag and drop on the spot.
 * `pointerdown` in the capture phase catches it wherever it lands, and the release
 * is listened for on the window so a pointer let go outside the pill still clears
 * the hold.
 */
export function usePanelHitRegion(active: boolean): void {
  useEffect(() => {
    if (!active) return;
    let last: HitRegion | null = null;
    let disposed = false;

    // Surfaces come and go with the form the window takes, so the resize
    // observer follows whatever the last measurement found.
    let observed: HTMLElement[] = [];
    const resizes =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(() => poll.wake());
    const follow = (nodes: HTMLElement[]) => {
      if (!resizes) return;
      if (
        nodes.length === observed.length &&
        nodes.every((node, i) => node === observed[i])
      )
        return;
      resizes.disconnect();
      resizes.observe(document.documentElement);
      for (const node of nodes) resizes.observe(node);
      observed = nodes;
    };

    /** Measure and report; true when the region changed. */
    const report = () => {
      if (disposed) return false;
      const nodes = surfaceNodes(document);
      follow(nodes);
      const next = regionOf(nodes);
      if (sameRegion(last, next)) return false;
      last = next;
      void emit("assistant-hit-rect", payloadFor(next));
      return true;
    };
    const poll = createAdaptivePoll({ tick: report });
    const wake = () => poll.wake();

    const hold = (held: boolean) => {
      void emit("assistant-panel-hold", { held });
    };
    const onDown = () => {
      hold(true);
      wake();
    };
    const onUp = () => {
      hold(false);
      wake();
    };

    // Any render, class or style change, or text growing in the answer.
    const mutations = new MutationObserver(wake);
    mutations.observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      characterData: true,
    });

    poll.start();
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("pointerup", onUp, true);
    window.addEventListener("pointercancel", onUp, true);
    window.addEventListener("blur", onUp);
    window.addEventListener("resize", wake);
    for (const name of MOTION_EVENTS)
      document.addEventListener(name, wake, true);
    for (const name of POINTER_WAKE_EVENTS)
      document.addEventListener(name, wake, true);

    return () => {
      disposed = true;
      poll.stop();
      mutations.disconnect();
      resizes?.disconnect();
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("pointerup", onUp, true);
      window.removeEventListener("pointercancel", onUp, true);
      window.removeEventListener("blur", onUp);
      window.removeEventListener("resize", wake);
      for (const name of MOTION_EVENTS)
        document.removeEventListener(name, wake, true);
      for (const name of POINTER_WAKE_EVENTS)
        document.removeEventListener(name, wake, true);
      hold(false);
    };
  }, [active]);
}

/**
 * Suppress the web view's own context menu in the panel — the window-wide rule
 * in `lib/contextMenu.ts`, kept as a hook so the panel states it where it sets
 * up the rest of its pointer handling.
 *
 * On an ordinary app window the built-in menu is merely out of place. On a
 * transparent HUD it is the tell that gave the bug away: a right-click meant for
 * the desktop answered with Copy / Copy link to highlight / Print / Inspect, from
 * a window the user could not see. Pass-through means those clicks no longer
 * reach the webview at all, but a right-click on the pill itself would still
 * raise it, and a browser menu is not a sensible answer for a voice chip.
 */
export function useSuppressContextMenu(): void {
  useEffect(() => {
    suppressBrowserContextMenu();
  }, []);
}
