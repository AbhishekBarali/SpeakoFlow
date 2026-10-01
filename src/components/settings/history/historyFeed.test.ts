import { expect, test } from "bun:test";
import type { AssistantHistorySummary, HistoryEntry } from "@/bindings";
import {
  FLOW_HISTORY_MARKER,
  buildFeed,
  cleanMessageContent,
  matchesFilter,
  rowKind,
  visibleFilters,
} from "./historyFeed";

const entry = (over: Partial<HistoryEntry> = {}): HistoryEntry => ({
  id: 1,
  file_name: "a.wav",
  timestamp: 100,
  saved: false,
  title: "",
  transcription_text: "hello",
  post_processed_text: null,
  post_process_prompt: null,
  post_process_requested: false,
  dismissed: false,
  ...over,
});

const session = (
  over: Partial<AssistantHistorySummary> = {},
): AssistantHistorySummary => ({
  id: 1,
  timestamp: 50,
  updated_at: 200,
  title: "What is Apple?",
  message_count: 2,
  meeting_id: null,
  meeting_title: null,
  kind: "ask",
  preview: "Apple is a company.",
  ...over,
});

test("each row kind lands in its own tab and nowhere else", () => {
  const feed = buildFeed(
    [
      entry({ id: 1, timestamp: 10 }),
      entry({ id: 2, timestamp: 20, post_process_prompt: FLOW_HISTORY_MARKER }),
    ],
    [
      session({ id: 3, updated_at: 30 }),
      session({ id: 4, updated_at: 40, kind: "call" }),
    ],
  );
  expect(feed.map(rowKind)).toEqual(["call", "ask", "flow", "dictation"]);
  const only = (filter: Parameters<typeof matchesFilter>[1]) =>
    feed.filter((item) => matchesFilter(item, filter)).map(rowKind);
  expect(only("all")).toHaveLength(4);
  expect(only("recordings")).toEqual(["dictation"]);
  expect(only("flow")).toEqual(["flow"]);
  expect(only("asks")).toEqual(["ask"]);
  expect(only("calls")).toEqual(["call"]);
});

test("a conversation sorts by its last activity, not when it started", () => {
  const feed = buildFeed(
    [entry({ timestamp: 150 })],
    [session({ timestamp: 50, updated_at: 200 })],
  );
  expect(feed.map((item) => item.kind)).toEqual(["assistant", "transcription"]);
});

test("a switched-off feature has no tab", () => {
  expect(
    visibleFilters({ flowEnabled: false, assistantEnabled: true }),
  ).toEqual(["all", "recordings", "asks", "calls"]);
  expect(
    visibleFilters({ flowEnabled: true, assistantEnabled: false }),
  ).toEqual(["all", "recordings", "flow"]);
  expect(
    visibleFilters({ flowEnabled: false, assistantEnabled: false }),
  ).toEqual(["all", "recordings"]);
});

test("a question about a selection reads as the question, with a count", () => {
  const stored = [
    "The user has this text selected in another application:",
    "<selected_text>",
    "Photosynthesis is the process",
    "</selected_text>",
    "",
    "Their request about it: explain this simply",
    "[screenshot attached]",
    "[file attached: notes.txt]",
  ].join("\n");
  const clean = cleanMessageContent(stored);
  expect(clean.text).toBe("explain this simply");
  expect(clean.selectionChars).toBe("Photosynthesis is the process".length);
  expect(clean.screenshot).toBe(true);
  expect(clean.files).toEqual(["notes.txt"]);
});

test("an ordinary message passes through untouched", () => {
  const clean = cleanMessageContent("line one\nline two");
  expect(clean).toEqual({
    text: "line one\nline two",
    screenshot: false,
    files: [],
    selectionChars: 0,
  });
});
