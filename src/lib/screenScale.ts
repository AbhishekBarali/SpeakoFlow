/**
 * Pick the main window's type scale from the display it is on.
 *
 * The settings UI is sized in rem, so one root font-size scales all of it. A
 * fixed 15px root looked right on a 1080p laptop and tiny on a 1440p monitor at
 * 100% scaling, where the window is usually maximized: 13px labels in a column
 * with a metre of empty space either side. Keying the step on the screen rather
 * than the window keeps text from jumping while a window is being resized.
 *
 * The shorter screen edge in CSS px is the signal because it already folds in
 * the OS scale factor: a 4K panel at 150% and a 1440p panel at 100% both report
 * 1440, and both have the same room.
 */
export type ScreenStep = "compact" | "default" | "large" | "huge";

export const screenStepFor = (width: number, height: number): ScreenStep => {
  const shortEdge = Math.min(width, height);
  if (!Number.isFinite(shortEdge) || shortEdge <= 0) return "default";
  if (shortEdge < 860) return "compact";
  if (shortEdge < 1300) return "default";
  if (shortEdge < 1800) return "large";
  return "huge";
};

/** Write the step for the current screen to `<html data-screen>`. */
export const applyScreenScale = (): void => {
  if (typeof window === "undefined" || typeof document === "undefined") return;
  const step = screenStepFor(window.screen.width, window.screen.height);
  if (document.documentElement.dataset.screen !== step) {
    document.documentElement.dataset.screen = step;
  }
};

/**
 * Re-check on resize: dragging the window to another monitor changes
 * `window.screen`, and a resize is the cheapest event that reliably follows.
 */
export const watchScreenScale = (): (() => void) => {
  if (typeof window === "undefined") return () => {};
  applyScreenScale();
  window.addEventListener("resize", applyScreenScale);
  return () => window.removeEventListener("resize", applyScreenScale);
};
