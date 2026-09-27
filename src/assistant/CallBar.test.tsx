import { afterEach, beforeEach, expect, jest, mock, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { ReactTestInstance } from "react-test-renderer";

/**
 * The call bar at rest, and the text box's way back to voice.
 *
 * Presentational, so it is checked directly with a stand-in voice: no window,
 * no microphone, no backend.
 */

mock.module("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: "en" },
  }),
}));
mock.module("@/bindings", () => ({
  commands: {
    getAssistantHistoryEntries: async () => ({
      status: "ok",
      data: { entries: [], has_more: false },
    }),
  },
}));
mock.module("@tauri-apps/api/event", () => ({
  listen: async () => () => {},
}));

const { CallSurface, TYPING_RETURN_MS, typingReturnDelay } = await import(
  "./CallBar"
);
type CallForm = import("./useCallForm").CallForm;

type Voice = React.ComponentProps<typeof CallSurface>["voice"];
let renderer: ReactTestRenderer;
let sent: string[] = [];
let expandToggles = 0;
let newChats = 0;

const voice = (over: Partial<Voice> = {}): Voice =>
  ({
    open: true,
    phase: "listening",
    error: null,
    clearError() {},
    level: 0,
    pace: "natural",
    setPace() {},
    sensitivity: "normal",
    setSensitivity() {},
    muted: false,
    speakerOff: false,
    start: async () => {},
    end() {},
    toggleMute: async () => {},
    toggleSpeaker: async () => {},
    sendText: async (text: string) => {
      sent.push(text);
      return true;
    },
    stopReply() {},
    newConversation: async () => {
      newChats++;
      return true;
    },
    loadConversation: async () => true,
    setComposing() {},
    browserSink: {},
    ...over,
  }) as unknown as Voice;

const surface = (props: { voice?: Voice; form?: CallForm }) => (
  <CallSurface
    voice={props.voice ?? voice()}
    form={props.form ?? "bar"}
    onExpand={() => expandToggles++}
    onCollapse={() => {}}
    onEnd={() => {}}
    name="Flow"
    profilePicker={null}
    transcript={null}
    resizeHandles={null}
    hasConversation
    activity={null}
    voiceLoading={null}
    voiceFault={null}
    onDismissFault={() => {}}
  />
);

const render = (node: React.ReactElement) => {
  act(() => {
    renderer = create(node);
  });
};

/** Buttons inside `.call-bar`, by accessible label. */
const barButtons = () => {
  const bar = renderer.root.find(
    (node) =>
      node.type === "div" &&
      typeof node.props.className === "string" &&
      node.props.className.split(" ").includes("call-bar"),
  );
  return bar
    .findAllByType("button")
    .map((button) => button.props["aria-label"] as string);
};

const button = (label: string): ReactTestInstance =>
  renderer.root.find(
    (node) => node.type === "button" && node.props["aria-label"] === label,
  );

const inputs = () => renderer.root.findAllByType("input");

const type = (text: string) => {
  act(() => {
    inputs()[0].props.onChange({ target: { value: text } });
  });
};

beforeEach(() => {
  sent = [];
  expandToggles = 0;
  newChats = 0;
  jest.useFakeTimers();
});

afterEach(() => {
  act(() => renderer.unmount());
  jest.useRealTimers();
});

test("at rest the bar is type, microphone, hang up and speaker — nothing else", () => {
  render(surface({}));
  expect(barButtons()).toEqual([
    "assistant.conversation.type",
    "assistant.conversation.mute",
    "assistant.conversation.end",
    "assistant.conversation.speakerOff",
  ]);
  // The easy misclicks are off the bar entirely.
  const labels = renderer.root
    .findAllByType("button")
    .map((node) => node.props["aria-label"]);
  expect(labels).not.toContain("assistant.conversation.newChat");
  expect(labels).not.toContain("assistant.conversation.collapse");
});

test("the pen is the keyboard", () => {
  render(surface({}));
  expect(inputs()).toHaveLength(0);
  act(() => button("assistant.conversation.type").props.onClick());
  expect(inputs()).toHaveLength(1);
});

test("after sending, the bar goes back to voice on its own", () => {
  render(surface({}));
  act(() => button("assistant.conversation.type").props.onClick());
  type("and in Nepali?");
  act(() => button("assistant.conversation.send").props.onClick());
  expect(sent).toEqual(["and in Nepali?"]);
  expect(inputs()).toHaveLength(1);
  act(() => jest.advanceTimersByTime(TYPING_RETURN_MS.afterSend - 1));
  expect(inputs()).toHaveLength(1);
  act(() => jest.advanceTimersByTime(1));
  expect(inputs()).toHaveLength(0);
});

test("an empty text box left alone goes back to voice", () => {
  render(surface({}));
  act(() => button("assistant.conversation.type").props.onClick());
  act(() => jest.advanceTimersByTime(TYPING_RETURN_MS.idleEmpty));
  expect(inputs()).toHaveLength(0);
});

test("a half-written message holds the box open longer, and survives it closing", () => {
  render(surface({}));
  act(() => button("assistant.conversation.type").props.onClick());
  type("remind me to");
  act(() => jest.advanceTimersByTime(TYPING_RETURN_MS.idleEmpty));
  expect(inputs()).toHaveLength(1);
  act(() => jest.advanceTimersByTime(TYPING_RETURN_MS.idleDraft));
  expect(inputs()).toHaveLength(0);
  act(() => button("assistant.conversation.type").props.onClick());
  expect(inputs()[0].props.value).toBe("remind me to");
});

test("typing restarts the clock", () => {
  render(surface({}));
  act(() => button("assistant.conversation.type").props.onClick());
  act(() => jest.advanceTimersByTime(TYPING_RETURN_MS.idleEmpty - 100));
  type("h");
  type("");
  act(() => jest.advanceTimersByTime(TYPING_RETURN_MS.idleEmpty - 100));
  expect(inputs()).toHaveLength(1);
});

test("speaking into an empty text box goes back to voice", () => {
  render(surface({}));
  act(() => button("assistant.conversation.type").props.onClick());
  act(() => renderer.update(surface({ voice: voice({ phase: "hearing" }) })));
  expect(inputs()).toHaveLength(0);
});

test("the bubble is the way into the conversation, and New chat lives there", () => {
  render(surface({}));
  act(() => button("assistant.conversation.expand").props.onClick());
  expect(expandToggles).toBe(1);
  act(() => renderer.update(surface({ form: "open" })));
  act(() => button("assistant.conversation.newChat").props.onClick());
  expect(newChats).toBe(1);
});

/** Every text node rendered inside `.call-bubble`. */
const bubbleText = () =>
  renderer.root
    .findAll(
      (node) =>
        node.type === "div" &&
        typeof node.props.className === "string" &&
        node.props.className.split(" ").includes("call-bubble"),
    )
    .flatMap((node) => node.findAllByType("span"))
    .flatMap((span) => span.children)
    .filter((child): child is string => typeof child === "string");

test("the bubble says what the assistant is doing, never the reply itself", () => {
  render(surface({ voice: voice({ phase: "responding" }) }));
  expect(bubbleText()).toContain("assistant.conversation.phase.responding");
  act(() => renderer.update(surface({ voice: voice({ phase: "speaking" }) })));
  expect(bubbleText()).toContain("assistant.conversation.phase.speaking");
});

test("the call options no longer offer the pause length", () => {
  // The popover listens for an outside click on `window`; the test runtime has
  // no DOM, so it gets just enough of one.
  const g = globalThis as { window?: unknown };
  const hadWindow = "window" in g;
  if (!hadWindow)
    g.window = { addEventListener() {}, removeEventListener() {} };
  try {
    render(surface({ form: "open" }));
    act(() => button("assistant.conversation.options").props.onClick());
    const selects = renderer.root.findAllByType("select");
    expect(selects.map((node) => node.props.id)).toEqual(["call-sensitivity"]);
    act(() => renderer.unmount());
    render(surface({}));
  } finally {
    if (!hadWindow) delete g.window;
  }
});

test("the return delays are ordered the way the behaviour needs", () => {
  expect(typingReturnDelay("", true)).toBe(TYPING_RETURN_MS.afterSend);
  expect(typingReturnDelay("", false)).toBe(TYPING_RETURN_MS.idleEmpty);
  expect(typingReturnDelay("  hi ", true)).toBe(TYPING_RETURN_MS.idleDraft);
  expect(TYPING_RETURN_MS.afterSend).toBeLessThan(TYPING_RETURN_MS.idleEmpty);
  expect(TYPING_RETURN_MS.idleEmpty).toBeLessThan(TYPING_RETURN_MS.idleDraft);
});
