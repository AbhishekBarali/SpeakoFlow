import { describe, expect, mock, test } from "bun:test";

// bun:test shares one module registry across test files, so a mock replaces
// the module for every file that runs after this one. `isTauri` is part of the
// mock for that reason: localVoice.ts imports it, and without it that suite
// fails with "Export named 'isTauri' not found" depending on file order.
mock.module("@tauri-apps/api/core", () => ({
  invoke: async () => {},
  isTauri: () => false,
}));
mock.module("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    startDragging: async () => {},
    startResizeDragging: async () => {},
  }),
}));

const { createDragGesture } = await import("./useSafeWindowDrag");

const press = (
  over: Partial<
    Parameters<ReturnType<typeof createDragGesture>["press"]>[0]
  > = {},
) => ({
  button: 0,
  detail: 1,
  onSurface: true,
  x: 100,
  y: 100,
  ...over,
});

describe("a press on a drag surface", () => {
  test("a click never moves the window", () => {
    const gesture = createDragGesture({ maximizeOnDoubleClick: false });
    expect(gesture.press(press())).toBe(true);
    // Hand tremor, under the threshold.
    expect(gesture.move({ buttons: 1, x: 102, y: 101 })).toBe(false);
    gesture.release();
    // A click that was never a drag reaches its target.
    expect(gesture.click()).toBe(false);
  });

  test("moves once, when the pointer has travelled far enough", () => {
    const gesture = createDragGesture({ maximizeOnDoubleClick: false });
    gesture.press(press());
    expect(gesture.move({ buttons: 1, x: 104, y: 100 })).toBe(true);
    // The window is already moving; further motion is the move itself.
    expect(gesture.move({ buttons: 1, x: 140, y: 120 })).toBe(false);
  });

  test("the click that ends a drag is swallowed, and only that one", () => {
    const gesture = createDragGesture({ maximizeOnDoubleClick: false });
    gesture.press(press());
    gesture.move({ buttons: 1, x: 130, y: 100 });
    gesture.release();
    // Released over the pill's cancel button: that is not a cancel.
    expect(gesture.click()).toBe(true);
    // The next real click goes through.
    expect(gesture.click()).toBe(false);
  });

  test("a drag whose release never arrived cannot eat the next press's click", () => {
    const gesture = createDragGesture({ maximizeOnDoubleClick: false });
    gesture.press(press());
    gesture.move({ buttons: 1, x: 130, y: 100 });
    // No release and no click (a system loop swallowed both). A fresh press on
    // a button inside the panel must still land.
    gesture.press(press({ onSurface: false }));
    expect(gesture.click()).toBe(false);
  });

  test("a button already released is not the start of a drag", () => {
    const gesture = createDragGesture({ maximizeOnDoubleClick: false });
    gesture.press(press());
    expect(gesture.move({ buttons: 0, x: 200, y: 200 })).toBe(false);
    expect(gesture.move({ buttons: 1, x: 300, y: 300 })).toBe(false);
  });

  test("a double click is kept from Tauri, which would maximize the window", () => {
    const floating = createDragGesture({ maximizeOnDoubleClick: false });
    expect(floating.press(press({ detail: 2 }))).toBe(true);
    const titleBar = createDragGesture({ maximizeOnDoubleClick: true });
    expect(titleBar.press(press({ detail: 2 }))).toBe(false);
  });

  test("presses off a drag surface, or with another button, are left alone", () => {
    const gesture = createDragGesture({ maximizeOnDoubleClick: false });
    expect(gesture.press(press({ onSurface: false }))).toBe(false);
    expect(gesture.move({ buttons: 1, x: 200, y: 100 })).toBe(false);
    expect(gesture.press(press({ button: 2 }))).toBe(false);
    expect(gesture.move({ buttons: 2, x: 200, y: 100 })).toBe(false);
  });
});
