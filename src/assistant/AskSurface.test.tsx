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
const { HIT_SURFACE_SELECTORS } = await import("./hitRegion");

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
  onCancel: noop,
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
  // One control, and only one: a × that cancels. There is no confirm button —
  // releasing the shortcut still sends — because a second target is what made this
  // surface look bulkier than the dictation pill it mirrors. But there is always a
  // way out, because "press the key again" is a fact you have to be told, not an
  // affordance.
  const labels = renderer.root
    .findAllByType("button")
    .map((button) => button.props["aria-label"]);
  expect(labels).toEqual(["assistant.cancel"]);
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
    // Cancel is present in every working phase, and is the only control in any of
    // them — same glyph, same place, so it is learned once.
    const labels = renderer.root
      .findAllByType("button")
      .map((button) => button.props["aria-label"]);
    expect(labels).toEqual(["assistant.cancel"]);
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
  // One way out, which in this phase stops the reply rather than merely hiding it:
  // a hidden window with a reply still generating is the same class of bug as a
  // call outliving its panel.
  const labels = renderer.root
    .findAllByType("button")
    .map((button) => button.props["aria-label"]);
  expect(labels).toEqual(["assistant.cancel"]);
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

/**
 * The measured-surface list must actually contain the surface these components
 * draw, and that link needs a test rather than a reader's attention.
 *
 * `hitRegion.ts` reports the union of the surfaces it can find to Rust, and
 * everything outside that union is handed to whatever is behind the window. So a
 * visible surface missing from the list is not a partial failure — it makes the
 * whole window cursor pass-through, and the ask surface is exactly what that
 * happened to. `.ask-pill` was absent, the ask stage keeps both layers mounted so
 * the only listed element was the faded-out card, nothing measured as drawn, and
 * the surface the assistant hotkey opens took no clicks at all: no cancel, no
 * typing, no drag, no close. Because the report is change-gated it was sent once
 * and never revised, so it stayed dead for as long as the panel was up.
 *
 * Asserting the rendered class name against the list is what makes a rename on
 * either side fail here instead of on a user's desktop.
 */
test("every surface these components draw is one hitRegion measures", () => {
  const listed = new Set(
    HIT_SURFACE_SELECTORS.filter((selector) => selector.startsWith(".")).map(
      (selector) => selector.slice(1),
    ),
  );

  const rootIsMeasured = (label: string) => {
    const root = classes()[0] ?? "";
    const tokens = root.split(/\s+/).filter(Boolean);
    expect(
      tokens.some((token) => listed.has(token)),
      `${label} renders "${root}", none of which is in HIT_SURFACE_SELECTORS, so the whole panel window would go pass-through`,
    ).toBe(true);
  };

  for (const phase of [
    "listening",
    "transcribing",
    "working",
    "prompt",
  ] as const) {
    render(<AskBar {...barProps} phase={phase} />);
    rootIsMeasured(`AskBar in the ${phase} phase`);
    act(() => renderer.unmount());
  }

  render(
    <AskCard
      question="q"
      answer="a"
      busy={false}
      status="assistant.status.thinking"
      markdown={{}}
      onClose={noop}
      stopDrag={noop}
    />,
  );
  rootIsMeasured("AskCard");
});
