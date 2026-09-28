import { afterEach, expect, mock, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import {
  parseLayout,
  quickAskPhase,
  quickAskShape,
  workingLabelKey,
} from "./quickAskState";

/**
 * The quick ask: one question, one answer, and the surface that shows them.
 *
 * The state derivation is the part that decides whether the surface feels alive
 * or stuck, and the component is presentational, so both are checked directly —
 * no window, no backend, no pipeline.
 */

mock.module("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const { default: QuickAsk } = await import("./QuickAsk");
const { HIT_SURFACE_SELECTORS } = await import("./hitRegion");

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

const idle = {
  state: "idle",
  stream: "",
  answer: "",
  hasError: false,
} as const;

test("the first token opens the card, instead of the finished answer", () => {
  // A long answer used to spend its whole generation behind a spinner, because
  // the card only opened once the reply was complete and measured.
  expect(quickAskPhase({ ...idle, state: "thinking", stream: "The" })).toBe(
    "answering",
  );
  expect(quickAskShape("answering")).toBe("card");
});

test("waiting is a pill, typing is a bar, reading is a card", () => {
  expect(quickAskPhase({ ...idle, state: "listening" })).toBe("listening");
  expect(quickAskPhase({ ...idle, state: "transcribing" })).toBe(
    "transcribing",
  );
  expect(quickAskPhase({ ...idle, state: "searching" })).toBe("working");
  expect(quickAskPhase(idle)).toBe("prompt");
  expect(quickAskPhase({ ...idle, answer: "Done." })).toBe("done");

  expect(quickAskShape("listening")).toBe("pill");
  expect(quickAskShape("transcribing")).toBe("pill");
  expect(quickAskShape("working")).toBe("pill");
  expect(quickAskShape("prompt")).toBe("bar");
  expect(quickAskShape("done")).toBe("card");
  expect(quickAskShape("error")).toBe("card");
});

test("a new question being spoken outranks the previous answer", () => {
  expect(
    quickAskPhase({ ...idle, state: "listening", answer: "Old answer" }),
  ).toBe("listening");
});

test("an error shows once the pipeline has stopped", () => {
  expect(quickAskPhase({ ...idle, hasError: true })).toBe("error");
  // A new turn clears errors as it starts; until then the turn owns the surface.
  expect(quickAskPhase({ ...idle, state: "thinking", hasError: true })).toBe(
    "working",
  );
});

test("the waiting label is one of four short states", () => {
  expect(workingLabelKey("listening", "listening", null)).toBe(
    "assistant.status.listening",
  );
  expect(workingLabelKey("working", "thinking", null)).toBe(
    "assistant.status.thinking",
  );
  expect(workingLabelKey("working", "searching", null)).toBe(
    "assistant.status.searching",
  );
  // A tool that changes what to expect is named, and beats the pipeline state.
  expect(
    workingLabelKey("working", "thinking", { name: "web_search", detail: "" }),
  ).toBe("assistant.status.searching");
  expect(
    workingLabelKey("working", "thinking", {
      name: "capture_screen",
      detail: "",
    }),
  ).toBe("assistant.tool.screen");
  // Transcription and every other tool are, to the person waiting, thinking.
  // A pill that flips through five labels in two seconds reads as noise.
  expect(workingLabelKey("transcribing", "transcribing", null)).toBe(
    "assistant.status.thinking",
  );
  expect(
    workingLabelKey("working", "thinking", {
      name: "set_reminder",
      detail: "",
    }),
  ).toBe("assistant.status.thinking");
});

test("a malformed layout falls back to the top centre", () => {
  expect(parseLayout({ align: "bottom", justify: "end" })).toEqual({
    align: "bottom",
    justify: "end",
  });
  expect(parseLayout(null)).toEqual({ align: "top", justify: "center" });
  expect(parseLayout({ align: "sideways", justify: 3 })).toEqual({
    align: "top",
    justify: "center",
  });
});

// ---------------------------------------------------------------------------
// The surface
// ---------------------------------------------------------------------------

let renderer: ReactTestRenderer | null = null;
const noop = () => {};

const base = {
  status: "assistant.status.thinking",
  question: "",
  answer: "",
  selectionChars: 0,
  screen: false,
  error: null,
  notice: null,
  canRetry: false,
  input: "",
  onInputChange: noop,
  onSubmit: noop,
  onClose: noop,
  onCancel: noop,
  onStop: noop,
  onRetry: noop,
  onInsert: async () => true,
};

const render = (node: React.ReactElement) => {
  act(() => {
    renderer = create(node);
  });
  return renderer!;
};

afterEach(() => {
  if (renderer) act(() => renderer!.unmount());
  renderer = null;
});

const labels = () =>
  renderer!.root
    .findAllByType("button")
    .map((button) => button.props["aria-label"] ?? button.props.title);

const surface = () =>
  renderer!.root.find(
    (node) =>
      typeof node.type === "string" && node.props.className === "qa-surface",
  );

test("there is no follow-up field once an answer is on screen", () => {
  render(
    <QuickAsk
      {...base}
      phase="done"
      question="Translate this"
      answer="Traduce esto"
    />,
  );
  // One question, one answer: a conversation is what the call is for.
  expect(renderer!.root.findAllByType("input")).toHaveLength(0);
  expect(surface().props["data-shape"]).toBe("card");
});

test("a finished answer offers Copy and Insert, and the way out is Close", () => {
  render(<QuickAsk {...base} phase="done" question="q" answer="a" canRetry />);
  expect(labels()).toEqual([
    "common.close",
    "assistant.conversation.retry",
    "assistant.copy",
    "assistant.insert",
  ]);
});

test("an answer about a selection offers to replace it", () => {
  render(
    <QuickAsk
      {...base}
      phase="done"
      question="Make this shorter"
      answer="Shorter."
      selectionChars={240}
    />,
  );
  expect(labels()).toContain("assistant.insertReplace");
  const texts = renderer!.root
    .findAll((node) => typeof node.type === "string")
    .flatMap((node) => node.children)
    .filter((child): child is string => typeof child === "string");
  expect(texts).toContain("assistant.quick.selection");
});

test("while it writes, Stop keeps the answer and the × cancels it", () => {
  render(
    <QuickAsk {...base} phase="answering" question="q" answer="Partial" />,
  );
  expect(labels()).toEqual(["assistant.cancel", undefined]);
  const stop = renderer!.root
    .findAllByType("button")
    .find(
      (button) =>
        button.findAll((n) => n.children.includes("assistant.stop")).length > 0,
    );
  expect(stop).toBeDefined();
});

test("every waiting state has exactly one control, the cancel", () => {
  for (const phase of ["listening", "transcribing", "working"] as const) {
    render(<QuickAsk {...base} phase={phase} levels={[0.4]} />);
    expect(surface().props["data-shape"]).toBe("pill");
    expect(labels()).toEqual(["assistant.cancel"]);
    act(() => renderer!.unmount());
    renderer = null;
  }
});

const classNames = () =>
  renderer!.root
    .findAll((node) => typeof node.type === "string")
    .map((node) => String(node.props.className ?? ""));

/**
 * The waiting pill is the call's status bubble: level bars while listening, a
 * spinner otherwise, then the words. It used to lead with a teal sparkle badge
 * and a spinning ring, which is what made it read as a different product.
 */
test("the pill is the call bubble's indicator and words, with no badge", () => {
  render(<QuickAsk {...base} phase="listening" levels={[0.5]} />);
  expect(classNames()).toContain("call-level");
  expect(classNames().some((c) => c.includes("qa-mark"))).toBe(false);
  act(() => renderer!.unmount());
  renderer = null;

  render(<QuickAsk {...base} phase="working" />);
  expect(classNames().some((c) => c.includes("call-spin"))).toBe(true);
  expect(classNames()).not.toContain("call-level");
});

/**
 * What the answer is about belongs on the card, where it explains the answer.
 * On the pill "Screen" and "Selection" were chips beside "Listening…" — noise
 * about something the user did a second ago.
 */
test("selection and screen chips appear on the card, never on the pill", () => {
  for (const phase of ["listening", "transcribing", "working"] as const) {
    render(<QuickAsk {...base} phase={phase} selectionChars={40} screen />);
    expect(classNames()).not.toContain("qa-chip");
    act(() => renderer!.unmount());
    renderer = null;
  }
  render(
    <QuickAsk
      {...base}
      phase="done"
      question="q"
      answer="a"
      selectionChars={40}
      screen
    />,
  );
  expect(classNames().filter((c) => c === "qa-chip")).toHaveLength(2);
});

test("an error is announced and carries no attachment chips", () => {
  render(
    <QuickAsk
      {...base}
      phase="error"
      error="The provider refused"
      screen
      selectionChars={12}
    />,
  );
  expect(classNames()).not.toContain("qa-chip");
  const alert = renderer!.root.find(
    (node) => typeof node.type === "string" && node.props.role === "alert",
  );
  expect(alert.children).toContain("The provider refused");
});

test("the prompt is a field with send, and send is refused while empty", () => {
  render(<QuickAsk {...base} phase="prompt" />);
  expect(renderer!.root.findAllByType("input")).toHaveLength(1);
  const send = renderer!.root
    .findAllByType("button")
    .find((button) => button.props["aria-label"] === "assistant.send");
  expect(send?.props.disabled).toBe(true);
});

test("an error without a question to retry is one line and a close", () => {
  render(<QuickAsk {...base} phase="error" error="No microphone found" />);
  expect(labels()).toEqual(["common.close"]);
  const texts = renderer!.root
    .findAll((node) => typeof node.type === "string")
    .flatMap((node) => node.children)
    .filter((child): child is string => typeof child === "string");
  expect(texts).toContain("No microphone found");
});

/**
 * The measured-surface list must contain the surface this component draws.
 * `hitRegion.ts` hands everything outside the listed surfaces to whatever is
 * behind the window, so a rename on either side must fail here rather than on a
 * user's desktop.
 */
test("the surface is one hitRegion measures", () => {
  render(<QuickAsk {...base} phase="listening" />);
  const listed = HIT_SURFACE_SELECTORS.filter((s) => s.startsWith(".")).map(
    (s) => s.slice(1),
  );
  expect(listed).toContain(surface().props.className);
});
