import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Pixels the pointer must travel while held before a window drag begins. */
const DRAG_THRESHOLD_PX = 4;

const DRAG_ATTR = "data-tauri-drag-region";

/**
 * Things that are never a drag handle, however deep inside a drag region they sit.
 *
 * The ancestor match below is what makes a drag region behave the way it looks, and
 * this is the other half of it: a control inside one has to stay a control. Every
 * interactive element in these windows is in this list, and a press on one sets no
 * drag origin at all — so it cannot be turned into a window move by a hand that
 * shifts four pixels between press and release.
 */
const NEVER_DRAGGABLE = [
  "button",
  "a",
  "input",
  "textarea",
  "select",
  "label",
  "[role='button']",
  "[role='slider']",
  "[contenteditable='true']",
].join(", ");

/**
 * Window dragging that does not wedge Windows.
 *
 * Tauri's own `data-tauri-drag-region` handler starts the drag on `mousedown`
 * with no movement at all (`src/window/scripts/drag.js`), and on Windows
 * `start_dragging` is `ReleaseCapture()` plus a synthetic `WM_NCLBUTTONDOWN`
 * on the caption. That hands the mouse to Windows' modal move loop, which
 * swallows the `mouseup`: the window can stay stuck following the cursor until
 * the next click, and while it is stuck it owns the mouse, so clicks on other
 * applications do nothing. That is
 * https://github.com/tauri-apps/tauri/issues/10767, confirmed by the
 * maintainers and not fixable from the app side while WebView2 runs in hosted
 * mode.
 *
 * What is fixable is the trigger. A plain click never needs to enter the move
 * loop, so this waits until the pointer has actually travelled
 * {@link DRAG_THRESHOLD_PX} while held and only then calls `startDragging`.
 * Clicking a draggable surface, which is what the assistant pill does on every
 * interaction, no longer touches the loop at all.
 *
 * The listener sits on `document` in the capture phase so it runs before
 * Tauri's own bubble-phase listener and can take the event away from it with
 * `stopImmediatePropagation`. Only elements carrying the drag attribute are
 * intercepted, and those are inert containers, so no React handler is lost.
 */
export function useSafeWindowDrag(): void {
  useEffect(() => {
    let origin: { x: number; y: number } | null = null;

    /**
     * Is this press on a surface that may move the window?
     *
     * Matched against the pressed element's **ancestors**, not just the element
     * itself. Asking only about the target meant a drag region was a handle for its
     * own background and nothing else: every inert child had to repeat the attribute
     * to be draggable, which is why the collapsed pill carries nine copies of it —
     * and it is why the call view, whose header is styled `cursor: grab` across its
     * full width, actually dragged only from the word "Conversation". A surface that
     * shows a grab cursor and then refuses to move is worse than one that never
     * offered.
     *
     * `NEVER_DRAGGABLE` is checked first and checked from the target upward, so a
     * control inside a drag region is still a control. `closest` is used for both
     * tests, which means the nearer of the two wins: a button inside a drag region is
     * a button, and an inert span inside that button is still part of the button.
     */
    const isDragSurface = (target: EventTarget | null): boolean => {
      if (!(target instanceof Element)) return false;
      if (target.closest(NEVER_DRAGGABLE)) return false;
      const region = target.closest(`[${DRAG_ATTR}]`);
      if (!region) return false;
      return region.getAttribute(DRAG_ATTR) !== "false";
    };

    const onMouseDown = (event: MouseEvent) => {
      if (event.button !== 0 || !isDragSurface(event.target)) return;
      // A double click is Tauri's maximize gesture, not a drag, and it never
      // enters the move loop. Leave that one to Tauri's own handler.
      if (event.detail !== 1) return;
      // Keep Tauri's handler from starting the native drag on this press.
      event.stopImmediatePropagation();
      // Same reason Tauri does it: stop the text caret appearing mid-drag.
      event.preventDefault();
      origin = { x: event.clientX, y: event.clientY };
    };

    const onMouseMove = (event: MouseEvent) => {
      if (!origin) return;
      // A released button that we never saw come up (the move loop is exactly
      // how that happens) must not start a drag on the next stray move.
      if (!(event.buttons & 1)) {
        origin = null;
        return;
      }
      const movedFar =
        Math.abs(event.clientX - origin.x) >= DRAG_THRESHOLD_PX ||
        Math.abs(event.clientY - origin.y) >= DRAG_THRESHOLD_PX;
      if (!movedFar) return;
      origin = null;
      void getCurrentWindow().startDragging();
    };

    const cancel = () => {
      origin = null;
    };

    document.addEventListener("mousedown", onMouseDown, true);
    document.addEventListener("mousemove", onMouseMove, true);
    document.addEventListener("mouseup", cancel, true);
    // The move loop eats the mouseup, so the blur that follows is the only
    // reliable signal that the press is over.
    window.addEventListener("blur", cancel);

    return () => {
      document.removeEventListener("mousedown", onMouseDown, true);
      document.removeEventListener("mousemove", onMouseMove, true);
      document.removeEventListener("mouseup", cancel, true);
      window.removeEventListener("blur", cancel);
    };
  }, []);
}
