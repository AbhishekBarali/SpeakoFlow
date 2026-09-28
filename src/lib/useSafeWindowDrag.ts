import { useEffect, type MouseEvent as ReactMouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
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
 * Windows is the one platform whose system window drag can wedge the desktop
 * (see {@link useSafeWindowDrag}). Read from the user agent rather than the OS
 * plugin so the hook works in every window without any setup.
 */
const IS_WINDOWS =
  typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

type ResizeDirection = Parameters<
  ReturnType<typeof getCurrentWindow>["startResizeDragging"]
>[0];

export interface SafeWindowDragOptions {
  /**
   * Hand moves to the operating system's own move loop instead of moving the
   * window from the app. Only for a real application window, which people expect
   * to snap to screen edges. Floating surfaces must not use it on Windows: see
   * the hook's documentation for what that loop does to them.
   */
  systemMove?: boolean;
  /**
   * Let a double click on a drag region reach Tauri's own handler, which toggles
   * maximize. Off by default: a floating panel has nothing to maximize into, and
   * a maximized transparent always-on-top window covers the whole display with
   * something invisible that takes every click. A window that wants the gesture
   * normally handles it itself (see `TitleBar`).
   */
  maximizeOnDoubleClick?: boolean;
}

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
export function isDragSurface(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target.closest(NEVER_DRAGGABLE)) return false;
  const region = target.closest(`[${DRAG_ATTR}]`);
  if (!region) return false;
  return region.getAttribute(DRAG_ATTR) !== "false";
}

/**
 * Move the window with the pointer that is dragging it.
 *
 * On Windows the move is done by the app (`start_window_drag`, see
 * `src-tauri/src/window_drag.rs`) instead of by the system move loop, and even
 * a window that keeps the system loop has it started by the backend, which
 * only enters it while the button is physically down. The fallback covers a
 * backend that does not have the command yet.
 */
function beginWindowMove(systemMove: boolean): void {
  const appWindow = getCurrentWindow();
  if (IS_WINDOWS) {
    void invoke("start_window_drag", { system: systemMove }).catch(() =>
      appWindow.startDragging(),
    );
    return;
  }
  void appWindow.startDragging();
}

/**
 * The rules of a press on a drag surface, with no DOM in them so they can be
 * tested: when to take the press away from Tauri, when it has become a move, and
 * which click belongs to the move rather than to what it landed on.
 */
export function createDragGesture(options: { maximizeOnDoubleClick: boolean }) {
  let origin: { x: number; y: number } | null = null;
  // The release that ends a drag the app performed also produces a `click` on
  // whatever the pointer ended over. That click belongs to the drag.
  let swallowClick = false;
  return {
    /** A mousedown. True when it must be kept from Tauri's own handler. */
    press(event: {
      button: number;
      detail: number;
      onSurface: boolean;
      x: number;
      y: number;
    }): boolean {
      // A new press is a new gesture, whatever the last one left behind. A
      // system move loop eats the release, so its drag never produces the click
      // that would otherwise have cleared this.
      swallowClick = false;
      if (event.button !== 0 || !event.onSurface) return false;
      // Tauri's maximize gesture, for the one window that asks for it.
      if (event.detail === 2 && options.maximizeOnDoubleClick) return false;
      origin = { x: event.x, y: event.y };
      return true;
    },
    /** A mousemove. True exactly once per press, when the window should move. */
    move(event: { buttons: number; x: number; y: number }): boolean {
      if (!origin) return false;
      // A released button that we never saw come up (the move loop is exactly
      // how that happens) must not start a drag on the next stray move.
      if (!(event.buttons & 1)) {
        origin = null;
        return false;
      }
      const movedFar =
        Math.abs(event.x - origin.x) >= DRAG_THRESHOLD_PX ||
        Math.abs(event.y - origin.y) >= DRAG_THRESHOLD_PX;
      if (!movedFar) return false;
      origin = null;
      swallowClick = true;
      return true;
    },
    /** A mouseup, or anything else that ends the press. */
    release(): void {
      origin = null;
    },
    /** A click. True when it is the tail of a drag and must be swallowed. */
    click(): boolean {
      const swallow = swallowClick;
      swallowClick = false;
      return swallow;
    },
  };
}

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
 * Two things are done about it here:
 *
 * - **A plain click never starts a move.** The drag waits until the pointer has
 *   actually travelled {@link DRAG_THRESHOLD_PX} while held.
 * - **On Windows, a floating window is moved without the loop at all.** Even a
 *   real drag through the loop swallows the release, and the assistant panel
 *   stays click-blocking from `pointerdown` until a `pointerup` that then never
 *   comes. The backend follows the cursor while the button is physically held
 *   instead, so the webview gets its own release like any other.
 *
 * The listener sits on `document` in the capture phase so it runs before
 * Tauri's own bubble-phase listener and can take the event away from it with
 * `stopImmediatePropagation`. Only elements carrying the drag attribute are
 * intercepted, and those are inert containers, so no React handler is lost.
 */
export function useSafeWindowDrag(options: SafeWindowDragOptions = {}): void {
  const { systemMove = false, maximizeOnDoubleClick = false } = options;
  useEffect(() => {
    const gesture = createDragGesture({ maximizeOnDoubleClick });

    const onMouseDown = (event: MouseEvent) => {
      const intercept = gesture.press({
        button: event.button,
        detail: event.detail,
        onSurface: isDragSurface(event.target),
        x: event.clientX,
        y: event.clientY,
      });
      if (!intercept) return;
      // Keep Tauri's handler from starting the native drag — or, on a double
      // click, from maximizing a window that has no business being maximized.
      event.stopImmediatePropagation();
      // Same reason Tauri does it: stop the text caret appearing mid-drag.
      event.preventDefault();
    };

    const onMouseMove = (event: MouseEvent) => {
      if (
        gesture.move({
          buttons: event.buttons,
          x: event.clientX,
          y: event.clientY,
        })
      )
        beginWindowMove(systemMove);
    };

    const onClick = (event: MouseEvent) => {
      if (!gesture.click()) return;
      event.stopImmediatePropagation();
      event.preventDefault();
    };

    const release = () => gesture.release();

    document.addEventListener("mousedown", onMouseDown, true);
    document.addEventListener("mousemove", onMouseMove, true);
    document.addEventListener("mouseup", release, true);
    document.addEventListener("click", onClick, true);
    // The move loop eats the mouseup, so the blur that follows is the only
    // reliable signal that the press is over.
    window.addEventListener("blur", release);

    return () => {
      document.removeEventListener("mousedown", onMouseDown, true);
      document.removeEventListener("mousemove", onMouseMove, true);
      document.removeEventListener("mouseup", release, true);
      document.removeEventListener("click", onClick, true);
      window.removeEventListener("blur", release);
    };
  }, [systemMove, maximizeOnDoubleClick]);
}

/**
 * Start a window resize from a resize grip, but only once the press has become a
 * drag.
 *
 * A resize runs through the same system loop as a move, with the same failure:
 * begun on the press itself, a quick click on a grip could leave the loop
 * running after the button was already up, and the window then resized with
 * the cursor — holding the mouse — until the next click. Waiting for
 * {@link DRAG_THRESHOLD_PX} of travel with the button held means the loop only
 * ever starts while a drag is genuinely under way.
 */
export function beginSafeResize(
  event: MouseEvent | ReactMouseEvent,
  direction: ResizeDirection,
): void {
  if (event.button !== 0) return;
  // Don't let the grip start a header drag or a text selection.
  event.preventDefault();
  event.stopPropagation();
  const start = { x: event.clientX, y: event.clientY };
  const finish = () => {
    window.removeEventListener("mousemove", onMove, true);
    window.removeEventListener("mouseup", finish, true);
    window.removeEventListener("blur", finish);
  };
  const onMove = (move: MouseEvent) => {
    if (!(move.buttons & 1)) {
      finish();
      return;
    }
    if (
      Math.abs(move.clientX - start.x) < DRAG_THRESHOLD_PX &&
      Math.abs(move.clientY - start.y) < DRAG_THRESHOLD_PX
    )
      return;
    finish();
    if (IS_WINDOWS) {
      // The backend only enters the resize loop while the button is still
      // physically down (see `start_window_resize`).
      void invoke("start_window_resize", { direction }).catch(() =>
        getCurrentWindow().startResizeDragging(direction),
      );
      return;
    }
    void getCurrentWindow().startResizeDragging(direction);
  };
  window.addEventListener("mousemove", onMove, true);
  window.addEventListener("mouseup", finish, true);
  window.addEventListener("blur", finish);
}
