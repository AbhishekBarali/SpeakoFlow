import { afterEach, beforeEach, expect, jest, mock, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

/**
 * The open and close of the call's conversation.
 *
 * Both forms share one window that never changes size, so the property under
 * test is that nothing waits on the native side: opening draws the panel in the
 * same render as the click, and closing is only the fold animation. A form that
 * waited on a geometry report is what made the call vanish for a beat.
 */

const reports: boolean[] = [];

mock.module("@/bindings", () => ({
  commands: {
    assistantConversationSetExpanded: async (expanded: boolean) => {
      reports.push(expanded);
    },
  },
}));

const { useCallForm, CALL_FORM_MS } = await import("./useCallForm");

let hook: ReturnType<typeof useCallForm>;
let renderer: ReactTestRenderer;

function Harness({ active }: { active: boolean }) {
  hook = useCallForm(active);
  return null;
}

beforeEach(async () => {
  reports.length = 0;
  jest.useFakeTimers();
  await act(async () => {
    renderer = create(<Harness active />);
  });
});

afterEach(() => {
  act(() => renderer.unmount());
  jest.useRealTimers();
});

test("opening draws the panel at once, with no blank beat to wait out", () => {
  expect(hook.form).toBe("bar");
  act(() => hook.expand());
  expect(hook.form).toBe("open");
  expect(hook.expanded).toBe(true);
  expect(reports).toEqual([true]);
});

test("closing folds the panel, then brings the bar back", () => {
  act(() => hook.expand());
  reports.length = 0;

  act(() => hook.collapse());
  expect(hook.form).toBe("closing");
  // Still mounted while it folds away, and Rust still sees it expanded.
  expect(hook.expanded).toBe(true);
  expect(reports).toEqual([]);
  act(() => jest.advanceTimersByTime(CALL_FORM_MS.close));
  expect(hook.form).toBe("bar");
  expect(hook.expanded).toBe(false);
  expect(reports).toEqual([false]);
});

test("double clicks report the change once", () => {
  act(() => {
    hook.expand();
    hook.expand();
  });
  expect(reports).toEqual([true]);
  act(() => {
    hook.collapse();
    hook.collapse();
  });
  act(() => jest.advanceTimersByTime(CALL_FORM_MS.close));
  expect(reports).toEqual([true, false]);
});

test("the bar cannot be collapsed and a fold cannot be re-opened midway", () => {
  act(() => hook.collapse());
  expect(hook.form).toBe("bar");
  act(() => hook.expand());
  act(() => hook.collapse());
  act(() => hook.expand());
  expect(hook.form).toBe("closing");
});

test("ending the call returns to the bar, with nothing left pending", async () => {
  act(() => hook.expand());
  act(() => hook.collapse());
  await act(async () => renderer.update(<Harness active={false} />));
  expect(hook.form).toBe("bar");
  act(() => jest.advanceTimersByTime(CALL_FORM_MS.close));
  expect(hook.form).toBe("bar");
  // The fold never finished, so it never reported.
  expect(reports).toEqual([true]);
});
