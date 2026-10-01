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
 * How often the drawn rect is re-measured.
 *
 * The quick ask's surface animates its width and grows as an answer streams in,
 * so this has to track a transition rather than just react to a render.
 * Measuring is a `getBoundingClientRect` and a `getComputedStyle` per surface;
 * the report is change-gated, so a still surface costs nothing beyond that.
 */
const MEASURE_INTERVAL_MS = 50;

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
function collectSurfaces(root: ParentNode): MeasuredSurface[] {
  return Array.from(root.querySelectorAll<HTMLElement>(HIT_SURFACES)).map(
    (node) => {
      const style = window.getComputedStyle(node);
      const rect = node.getBoundingClientRect();
      return {
        drawn: style.pointerEvents !== "none" && style.visibility !== "hidden",
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
      };
    },
  );
}

/** The union of everything the panel is drawing right now. */
export function measureHitRegion(root: ParentNode = document): HitRegion {
  return unionHitRect(collectSurfaces(root), window.devicePixelRatio || 1);
}

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

    const report = () => {
      if (disposed) return;
      const next = measureHitRegion();
      if (sameRegion(last, next)) return;
      last = next;
      void emit("assistant-hit-rect", payloadFor(next));
    };

    const hold = (held: boolean) => {
      void emit("assistant-panel-hold", { held });
    };
    const onDown = () => hold(true);
    const onUp = () => hold(false);

    report();
    const timer = window.setInterval(report, MEASURE_INTERVAL_MS);
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("pointerup", onUp, true);
    window.addEventListener("pointercancel", onUp, true);
    window.addEventListener("blur", onUp);

    return () => {
      disposed = true;
      window.clearInterval(timer);
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("pointerup", onUp, true);
      window.removeEventListener("pointercancel", onUp, true);
      window.removeEventListener("blur", onUp);
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
