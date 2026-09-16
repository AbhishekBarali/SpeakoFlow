import { afterEach, expect, mock, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

/**
 * What the quick ask shows before and after an answer.
 *
 * These two components are the whole visible difference between "the assistant
 * threw a big empty panel over my work and left it there" and "a bar listened,
 * then an answer arrived". They are presentational, so they can be checked
 * directly — no window, no backend, no pipeline.
 */

mock.module("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
mock.module("@/bindings", () => ({
  commands: { assistantInsertText: async () => ({ status: "ok" }) },
}));
// The waveform is exercised by its own tests; here it only needs to be
// identifiable, and stubbing it keeps the animation clock out of these cases.
mock.module("@/components/shared", () => ({
  AudioWaveform: (props: { mode?: string }) => (
    <div className="stub-waveform" data-mode={props.mode ?? "reactive"} />
  ),
}));

const { default: AskBar } = await import("./AskBar");
const { default: AskCard } = await import("./AskCard");

let renderer: ReactTestRenderer;
const noop = () => {};

const barProps = {
  active: true,
  status: "assistant.status.listening",
  question: "",
  input: "",
  onInputChange: noop,
  onSubmit: noop,
  onClose: noop,
  stopDrag: noop,
};

const render = (node: React.ReactElement) => {
  act(() => {
    renderer = create(node);
  });
  return renderer;
};

const texts = () =>
  renderer.root
    .findAll((node) => typeof node.type === "string")
    .flatMap((node) =>
      node.children.filter(
        (child): child is string => typeof child === "string",
      ),
    );

const classes = () =>
  renderer.root
    .findAll(
      (node) =>
        typeof node.type === "string" &&
        typeof node.props.className === "string",
    )
    .map((node) => node.props.className as string);

afterEach(() => {
  act(() => renderer.unmount());
});

test("listening is the bare dictation lozenge: a waveform, a mark, and no buttons", () => {
  render(
    <AskBar
      {...barProps}
      phase="listening"
      levels={[0.2, 0.6, 0.4]}
      preview=""
    />,
  );
  // A waveform, because the honest signal while the microphone is open is the
  // user's own voice moving.
  expect(
    renderer.root.findAllByProps({ className: "stub-waveform" }),
  ).toHaveLength(1);
  // The unlabelled shape, which is the one the user already knows from dictation:
  // no status prose competing with the wave for a 34px row.
  expect(classes()).toContain("ask-pill");
  expect(classes()).not.toContain("ask-pill labeled");
  // And no controls at all — the shortcut that started the recording ends it and
  // Esc discards it, exactly as with dictation. Cancel and confirm buttons were
  // most of what made this surface look bulkier than the one it mirrors.
  expect(renderer.root.findAllByType("button")).toHaveLength(0);
});

test("no working state widens the pill, and none of them shows prose", () => {
  // Listening, transcribing and thinking are one shape. Watching the surface
  // stretch while it works is worse than not knowing the word for what it is
  // doing, so the mark carries the state and the frame stays put.
  for (const phase of ["listening", "transcribing", "working"] as const) {
    render(
      <AskBar
        {...barProps}
        phase={phase}
        state={phase === "working" ? "searching" : undefined}
        status="assistant.status.thinking"
        levels={[0.2, 0.6, 0.4]}
      />,
    );
    expect(classes()).toContain("ask-pill");
    expect(classes()).not.toContain("ask-pill labeled");
    // The indicator is one fixed-width slot, so swapping the waveform for the
    // sweep mid-action cannot resize the pill.
    expect(classes()).toContain("ask-pill-indicator");
    // No status text, and so nothing that could lengthen the row.
    expect(texts()).not.toContain("assistant.status.thinking");
    expect(renderer.root.findAllByType("button")).toHaveLength(0);
    act(() => renderer.unmount());
  }
  // Re-render something so the shared afterEach has a tree to unmount.
  render(<AskBar {...barProps} phase="listening" />);
});

test("a failure is the one thing the pill will widen for, and it can be dismissed", () => {
  render(
    <AskBar
      {...barProps}
      phase="working"
      status="assistant.status.thinking"
      error="Something went wrong"
    />,
  );
  expect(texts()).toContain("Something went wrong");
  expect(classes()).toContain("ask-pill labeled");
  expect(classes()).toContain("ask-pill-label error");
  // Nothing will come along to replace a failure, so it gets a way out — unlike a
  // recording, which the shortcut ends and Esc discards.
  const labels = renderer.root
    .findAllByType("button")
    .map((button) => button.props["aria-label"]);
  expect(labels).toEqual(["assistant.hide"]);
});

test("the waiting indicator is the dictation overlay's, not a second one", () => {
  render(
    <AskBar {...barProps} phase="working" status="assistant.status.thinking" />,
  );
  // A step that cannot report a fraction should look the same everywhere in the
  // app, so this is `OverlayProgress` itself rather than a spinner invented for
  // this surface. Sharing the component is the point — a copy would drift.
  expect(classes()).toContain("overlay-progress");
  expect(classes()).toContain("progress-sheen");
  // No stop button: Esc interrupts a reply, as it does during a call.
  expect(renderer.root.findAllByType("button")).toHaveLength(0);
});

test("the prompt bar cannot take the caret while the card owns it", () => {
  render(<AskBar {...barProps} phase="prompt" active={false} />);
  const input = renderer.root.findByType("input");
  expect(input.props.disabled).toBe(true);
  // Send is refused too, so a keyboard cannot reach a surface at zero opacity.
  const send = renderer.root
    .findAllByType("button")
    .find((button) => button.props["aria-label"] === "assistant.send");
  expect(send?.props.disabled).toBe(true);
});

test("the answer card carries the answer, its actions, and no label over it", () => {
  render(
    <AskCard
      question="what does this mean"
      answer="It means exactly what it says."
      busy={false}
      status="assistant.status.thinking"
      markdown={{}}
      onClose={noop}
      stopDrag={noop}
    />,
  );
  const shown = texts();
  expect(shown).toContain("It means exactly what it says.");
  expect(shown).toContain("what does this mean");
  // The sparkle glyph and the word "Answer" are gone: a caption over the one
  // thing on screen that could not be anything else.
  expect(shown).not.toContain("assistant.answer");
  expect(classes()).not.toContain("ask-answer-head");
  // Copy, Insert and close — the three things worth doing with an answer.
  const labels = renderer.root
    .findAllByType("button")
    .map((button) => button.props["aria-label"]);
  expect(labels).toEqual([
    "assistant.copy",
    "assistant.insert",
    "assistant.hide",
  ]);
});

test("a follow-up being spoken withholds the previous exchange instead of looking answered", () => {
  render(
    <AskCard
      question="the previous question"
      answer="the previous answer"
      busy
      capturing
      status="assistant.status.listening"
      levels={[0.3]}
      markdown={{}}
      onClose={noop}
      stopDrag={noop}
    />,
  );
  const shown = texts();
  expect(shown).not.toContain("the previous answer");
  expect(shown).not.toContain("the previous question");
  expect(shown).toContain("assistant.status.listening");
});
