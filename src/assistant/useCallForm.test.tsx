import { afterEach, beforeEach, expect, jest, mock, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

/**
 * The open and close of the call's conversation, beat by beat.
 *
 * The lurch this exists to remove came from the window resizing while
 * something was drawn in it, so the property under test is ordering: the
 * window is only asked to change shape once the old form has gone, and the
 * new form only appears once the window reports that its geometry is in place.
 */

const resizes: boolean[] = [];
let frameListener: ((event: { payload: unknown }) => void) | null = null;

mock.module("@/bindings", () => ({
  commands: {
    assistantConversationSetExpanded: async (expanded: boolean) => {
      resizes.push(expanded);
    },
  },
}));
mock.module("@tauri-apps/api/event", () => ({
  listen: async (
    name: string,
    handler: (event: { payload: unknown }) => void,
  ) => {
    if (name === "assistant-call-frame") frameListener = handler;
    return () => {
      if (frameListener === handler) frameListener = null;
    };
  },
}));

const { useCallForm, CALL_FORM_MS } = await import("./useCallForm");

let hook: ReturnType<typeof useCallForm>;
let renderer: ReactTestRenderer;

function Harness({ active }: { active: boolean }) {
  hook = useCallForm(active);
  return null;
}

/** Rust applying the geometry and saying so. */
const report = async (expanded: boolean) => {
  await act(async () => frameListener?.({ payload: expanded }));
};

beforeEach(async () => {
  resizes.length = 0;
  jest.useFakeTimers();
  await act(async () => {
    renderer = create(<Harness active />);
  });
});

afterEach(() => {
  act(() => renderer.unmount());
  jest.useRealTimers();
});

test("opening resizes only after the bar has gone, and draws only after the resize", async () => {
  expect(hook.form).toBe("bar");
  act(() => hook.expand());
  expect(hook.form).toBe("leaving");
  expect(resizes).toEqual([]);
  act(() => jest.advanceTimersByTime(CALL_FORM_MS.leave));
  expect(hook.form).toBe("opening");
  expect(resizes).toEqual([true]);
  expect(hook.expanded).toBe(false);
  await report(true);
  expect(hook.form).toBe("open");
  expect(hook.expanded).toBe(true);
});

test("closing folds the panel first, then resizes, then brings the bar back", async () => {
  act(() => hook.expand());
  act(() => jest.advanceTimersByTime(CALL_FORM_MS.leave));
  await report(true);
  resizes.length = 0;

  act(() => hook.collapse());
  expect(hook.form).toBe("closing");
  // Still mounted while it folds away.
  expect(hook.expanded).toBe(true);
  expect(resizes).toEqual([]);
  act(() => jest.advanceTimersByTime(CALL_FORM_MS.close));
  expect(hook.form).toBe("shrinking");
  expect(resizes).toEqual([false]);
  await report(false);
  expect(hook.form).toBe("bar");
});

test("a geometry report that never comes does not strand the call", () => {
  act(() => hook.expand());
  act(() => jest.advanceTimersByTime(CALL_FORM_MS.leave));
  expect(hook.form).toBe("opening");
  act(() => jest.advanceTimersByTime(CALL_FORM_MS.fallback));
  expect(hook.form).toBe("open");
});

test("a report for the other direction is ignored", async () => {
  act(() => hook.expand());
  act(() => jest.advanceTimersByTime(CALL_FORM_MS.leave));
  await report(false);
  expect(hook.form).toBe("opening");
});

test("double clicks do not queue a second resize", () => {
  act(() => {
    hook.expand();
    hook.expand();
  });
  act(() => jest.advanceTimersByTime(CALL_FORM_MS.leave));
  expect(resizes).toEqual([true]);
  // Collapsing is only possible from the open form.
  act(() => hook.collapse());
  expect(hook.form).toBe("opening");
});

test("ending the call returns to the bar, with nothing left pending", async () => {
  act(() => hook.expand());
  await act(async () => renderer.update(<Harness active={false} />));
  expect(hook.form).toBe("bar");
  act(() =>
    jest.advanceTimersByTime(CALL_FORM_MS.leave + CALL_FORM_MS.fallback),
  );
  expect(hook.form).toBe("bar");
  expect(resizes).toEqual([]);
});
