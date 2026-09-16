/**
 * What the assistant panel is actually drawing, reported to Rust so the rest of
 * the window can pass clicks through to whatever is underneath it.
 *
 * The panel window is deliberately larger than its content: the pill hugs its own
 * text and floats centred in a transparent frame (340x56 of window for roughly
 * 155x34 of pill), which is what lets "Listening", "Thinking" and "Searching the
 * web" be different widths without a window resize between them. Invisible is not
 * the same as intangible, though — that surplus used to sit in front of the
 * desktop and swallow every click and right-click landing on it, which made the
 * screen feel dead whenever the assistant was up.
 *
 * Only the webview can say where the drawn edge is, so it measures and Rust
 * decides: see the cursor pass-through section in `assistant.rs`.
 */

import { emit } from "@tauri-apps/api/event";
import { useEffect } from "react";

/**
 * The surfaces that are genuinely visible and clickable in each form the window
 * takes. Everything else in the tree is a transparent frame or a centring box.
 *
 * `.apill-screen` is listed even though it lives inside `.apill`, because it is
 * positioned at `top: -5px` and an absolutely-positioned child that overflows its
 * parent contributes nothing to the parent's `getBoundingClientRect()`. Measuring
 * only the pill would leave the top of the screen-vision badge outside the tangible
 * area — and once armed, that badge is permanently visible and is the control that
 * says capture is on. Its own `pointer-events` still decides whether it counts, so
 * a hidden badge is correctly ignored.
 *
 * A form with none of these — the voice conversation view, the full chat panel —
 * measures as `unknown` and stays fully tangible. That is the behaviour that
 * shipped, so an unlisted form can never become unclickable; it just does not get
 * pass-through until it is listed.
 */
const HIT_SURFACES = ".apill, .apill-screen, .alive-card, .ask-card";

/**
 * How often the drawn rect is re-measured.
 *
 * The pill animates its width (a hover reveals expand/close over 260ms), so this
 * has to track a transition rather than just react to a render. Measuring is two
 * `getBoundingClientRect` calls and a `getComputedStyle`; the report is
 * change-gated, so a still pill costs nothing beyond the measurement.
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
 */
export type HitRegion =
  | { kind: "unknown" }
  | { kind: "none" }
  | { kind: "rect"; rect: HitRect };

/**
 * The union of every drawn surface, in PHYSICAL pixels.
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

  let left = Infinity;
  let top = Infinity;
  let right = -Infinity;
  let bottom = -Infinity;

  for (const surface of surfaces) {
    if (!surface.drawn) continue;
    if (surface.right <= surface.left || surface.bottom <= surface.top)
      continue;
    left = Math.min(left, surface.left);
    top = Math.min(top, surface.top);
    right = Math.max(right, surface.right);
    bottom = Math.max(bottom, surface.bottom);
  }

  if (!Number.isFinite(left) || !Number.isFinite(top)) return { kind: "none" };
  const scale = dpr > 0 ? dpr : 1;
  return {
    kind: "rect",
    rect: {
      x: left * scale,
      y: top * scale,
      width: (right - left) * scale,
      height: (bottom - top) * scale,
    },
  };
}

/**
 * Collect the drawn surfaces from the live document.
 *
 * `pointer-events` is the "is it drawn" test, and it is the right one rather than
 * a convenient one: it is inherited, and the stylesheet already sets it to `none`
 * on every layer that is faded out (`.ask-stage.leaving .ask-layer.pill`, an
 * inactive `.ask-layer.card`). So a surface mid-cross-fade is correctly reported as
 * not drawn, using the same declaration that stops it being clicked — one source of
 * truth instead of a second opacity rule to keep in sync.
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
      return region.rect;
    case "none":
      return { x: 0, y: 0, width: 0, height: 0 };
    case "unknown":
      return { tangible: true };
  }
}

function sameRegion(a: HitRegion | null, b: HitRegion): boolean {
  if (a === null || a.kind !== b.kind) return false;
  if (a.kind !== "rect" || b.kind !== "rect") return true;
  return (
    Math.abs(a.rect.x - b.rect.x) <= CHANGE_EPSILON &&
    Math.abs(a.rect.y - b.rect.y) <= CHANGE_EPSILON &&
    Math.abs(a.rect.width - b.rect.width) <= CHANGE_EPSILON &&
    Math.abs(a.rect.height - b.rect.height) <= CHANGE_EPSILON
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
 * Suppress WebView2's own context menu, except where it is the right answer.
 *
 * On an ordinary app window the built-in menu is harmless. On a transparent HUD it
 * is the tell that gave the bug away: a right-click meant for the desktop answered
 * with Copy / Copy link to highlight / Print / Inspect, from a window the user
 * could not see. Pass-through means those clicks no longer reach the webview at
 * all, but a right-click on the pill itself would still raise it, and a browser
 * menu is not a sensible answer for a voice chip.
 *
 * A text field is the exception, and not a grudging one: right-click to paste into
 * the prompt is something people reasonably expect, and blocking it there would
 * trade one small annoyance for another.
 */
export function useSuppressContextMenu(): void {
  useEffect(() => {
    const block = (event: MouseEvent) => {
      const target = event.target as Element | null;
      if (target?.closest?.("input, textarea, [contenteditable='true']"))
        return;
      event.preventDefault();
    };
    window.addEventListener("contextmenu", block);
    return () => window.removeEventListener("contextmenu", block);
  }, []);
}
