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
type Row = {
  id: number;
  timestamp: number;
  updated_at: number;
  title: string;
  message_count: number;
  meeting_id: number | null;
  meeting_title: string | null;
  kind: "ask" | "call";
  preview: string | null;
};
let historyRows: Row[] = [];
let listCalls: unknown[][] = [];
mock.module("@/bindings", () => ({
  commands: {
    getAssistantHistoryEntries: async () => ({
      status: "ok",
      data: { entries: [], has_more: false },
    }),
    listAssistantConversations: async (...args: unknown[]) => {
      listCalls.push(args);
      return { status: "ok", data: { entries: historyRows, has_more: false } };
    },
  },
}));
mock.module("@tauri-apps/api/event", () => ({
  emit: async () => {},
  listen: async () => () => {},
}));

const {
  CallSurface,
  MeetingStarter,
  TYPING_RETURN_MS,
  groupConversations,
  historyGroup,
  typingReturnDelay,
} = await import("./CallBar");
type CallForm = import("./useCallForm").CallForm;
type Meeting = NonNullable<React.ComponentProps<typeof CallSurface>["meeting"]>;

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
    branchConversation: async () => true,
    setComposing() {},
    browserSink: {},
    ...over,
  }) as unknown as Voice;

const surface = (props: {
  voice?: Voice;
  form?: CallForm;
  meeting?: Meeting | null;
}) => (
  <CallSurface
    voice={props.voice ?? voice()}
    form={props.form ?? "bar"}
    onExpand={() => expandToggles++}
    onCollapse={() => {}}
    onEnd={() => {}}
    name="Flow"
    profilePicker={null}
    transcript={null}
    meeting={props.meeting ?? null}
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
  historyRows = [];
  listCalls = [];
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

/* ── past conversations ── */

const seconds = (date: Date) => Math.floor(date.getTime() / 1000);

test("history groups by local calendar day, not by 24-hour windows", () => {
  const now = new Date(2026, 9, 1, 0, 30); // 00:30 on Oct 1
  expect(historyGroup(seconds(new Date(2026, 9, 1, 0, 5)), now)).toBe("today");
  // Two hours ago is yesterday at 00:30, not "today".
  expect(historyGroup(seconds(new Date(2026, 8, 30, 22, 30)), now)).toBe(
    "yesterday",
  );
  expect(historyGroup(seconds(new Date(2026, 8, 25, 12)), now)).toBe("week");
  expect(historyGroup(seconds(new Date(2026, 8, 24, 23)), now)).toBe("earlier");
});

test("grouping keeps the list's own order and only joins neighbours", () => {
  const now = new Date(2026, 9, 1, 12);
  const at = (date: Date) => ({ updated_at: seconds(date), timestamp: 0 });
  const groups = groupConversations(
    [
      at(new Date(2026, 9, 1, 11)),
      at(new Date(2026, 9, 1, 9)),
      at(new Date(2026, 8, 30, 9)),
      at(new Date(2026, 7, 1)),
    ],
    now,
  );
  expect(groups.map((g) => [g.group, g.entries.length])).toEqual([
    ["today", 2],
    ["yesterday", 1],
    ["earlier", 1],
  ]);
});

/** Run pending effects and the promises they started. */
const settle = async () => {
  await act(async () => {
    await Promise.resolve();
  });
};

test("past conversations can be narrowed to meetings, and show which meeting", async () => {
  historyRows = [
    {
      id: 3,
      timestamp: seconds(new Date()) - 60,
      updated_at: seconds(new Date()),
      title: "What should I send Priya?",
      message_count: 4,
      meeting_id: 9,
      meeting_title: "Pricing review",
      kind: "call",
      preview: null,
    },
  ];
  render(surface({ form: "open" }));
  act(() => button("assistant.conversation.historyOpen").props.onClick());
  await settle();
  expect(listCalls[0]?.[0]).toEqual({
    query: null,
    meetings_only: false,
    meeting_id: null,
    kind: "call",
  });

  const scope = renderer.root.find(
    (node) =>
      node.type === "button" &&
      node.props["aria-pressed"] === false &&
      node.children.includes("assistant.conversation.history.scopes.meetings"),
  );
  act(() => scope.props.onClick());
  await settle();
  expect(listCalls[listCalls.length - 1]?.[0]).toMatchObject({
    meetings_only: true,
  });

  const text = renderer.root
    .findAllByType("span")
    .flatMap((span) => span.children)
    .filter((child): child is string => typeof child === "string");
  expect(text).toContain("Pricing review");
  const headings = renderer.root
    .findAllByType("h3")
    .flatMap((heading) => heading.children);
  expect(headings).toEqual(["assistant.conversation.history.groups.today"]);
});

test("searching waits for typing to pause, and Escape clears before closing", async () => {
  render(surface({ form: "open" }));
  act(() => button("assistant.conversation.historyOpen").props.onClick());
  await settle();
  const search = () =>
    renderer.root.find(
      (node) => node.type === "input" && node.props.type === "search",
    );
  act(() => search().props.onChange({ target: { value: "pricing fri" } }));
  const before = listCalls.length;
  act(() => jest.advanceTimersByTime(100));
  await settle();
  expect(listCalls.length).toBe(before);
  act(() => jest.advanceTimersByTime(200));
  await settle();
  expect(listCalls[listCalls.length - 1]?.[0]).toMatchObject({
    query: "pricing fri",
  });

  let prevented = 0;
  const escape = () => ({
    key: "Escape",
    preventDefault: () => prevented++,
  });
  act(() => search().props.onKeyDown(escape()));
  expect(search().props.value).toBe("");
  act(() => search().props.onKeyDown(escape()));
  expect(prevented).toBe(2);
  // Closed: back to the conversation.
  expect(
    renderer.root.findAll(
      (node) => node.type === "input" && node.props.type === "search",
    ),
  ).toHaveLength(0);
});

/* ── a call about a meeting ── */

const pricing: Meeting = { meetingId: 9, title: "Pricing review" };

test("an open call about a meeting says which one", () => {
  render(surface({ form: "open", meeting: pricing }));
  const chip = renderer.root.find(
    (node) =>
      node.type === "div" && node.props.className === "call-meeting-chip",
  );
  expect(chip.findAllByType("span")[0].children).toEqual([
    "assistant.conversation.meeting.about",
  ]);
  act(() => renderer.update(surface({ form: "open", meeting: null })));
  expect(
    renderer.root.findAll(
      (node) => node.props.className === "call-meeting-chip",
    ),
  ).toHaveLength(0);
});

test("the meeting starter's chips ask their question", () => {
  const asked: string[] = [];
  render(
    <MeetingStarter
      meeting={pricing}
      onAsk={(text) => asked.push(text)}
      disabled={false}
    />,
  );
  const chips = renderer.root.findAllByType("button");
  expect(chips).toHaveLength(4);
  act(() => chips[0].props.onClick());
  expect(asked).toEqual(["assistant.conversation.meeting.prompts.recap"]);

  act(() =>
    renderer.update(
      <MeetingStarter meeting={pricing} onAsk={() => {}} disabled />,
    ),
  );
  expect(
    renderer.root.findAllByType("button").every((b) => b.props.disabled),
  ).toBe(true);
});
