import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
type EventHandler = (event: { payload: unknown }) => void;
const events = new Map<string, EventHandler>();
const calls: { command: string; args: unknown }[] = [];
let copyResult: Promise<void> = Promise.resolve();
let finishLanguage: () => void = () => {};
mock.module("@tauri-apps/api/event", () => ({
  listen: async (name: string, handler: EventHandler) => {
    events.set(name, handler);
    return () => events.delete(name);
  },
  emit: async () => {},
}));
mock.module("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => {
    calls.push({ command, args });
    return copyResult;
  },
}));
mock.module("@/i18n", () => ({
  default: { language: "en" },
  syncLanguageFromSettings: () =>
    new Promise<void>((resolve) => {
      finishLanguage = resolve;
    }),
}));
mock.module("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
const { default: RecordingOverlay } = await import("./RecordingOverlay");
let renderer: ReactTestRenderer;
const fire = async (name: string, payload: unknown = null) => {
  await act(async () => {
    events.get(name)?.({ payload });
  });
};
const rootClass = () =>
  renderer.root.findByProps({ dir: "ltr" }).props.className as string;
beforeEach(async () => {
  calls.length = 0;
  copyResult = Promise.resolve();
  events.clear();
  await act(async () => {
    renderer = create(<RecordingOverlay />, {
      createNodeMock: () => ({
        matches: () => false,
        scrollTop: 0,
        scrollHeight: 0,
      }),
    });
  });
});
afterEach(() => {
  act(() => renderer.unmount());
});
test("a delayed language sync cannot resurrect a hidden recording", async () => {
  await fire("show-overlay", { state: "recording", streamingWindow: false });
  await fire("hide-overlay");
  await act(async () => {
    finishLanguage();
  });
  expect(rootClass()).toContain("native-window-hidden");
  expect(renderer.root.findAllByType("button")).toHaveLength(0);
});
test("completion preserves the final cleaned text during fade and copies that text", async () => {
  await fire("show-overlay", { state: "recording", streamingWindow: true });
  await fire("stream-text", {
    committed: "um old words",
    tentative: " tentative",
  });
  await fire("finish-overlay", { epoch: 7, text: "The final cleaned words." });
  await fire("fade-overlay", 7);
  expect(rootClass()).toContain("is-fading");
  const buttons = renderer.root.findAllByType("button");
  expect(buttons).toHaveLength(1);
  await act(async () => {
    await buttons[0].props.onClick();
  });
  expect(calls).toEqual([
    {
      command: "copy_overlay_transcript",
      args: { text: "The final cleaned words." },
    },
  ]);
  expect(buttons[0].props["aria-label"]).toBe("overlay.copied");
  await fire("restore-overlay", 7);
  expect(rootClass()).not.toContain("is-fading");
  await fire("show-overlay", { state: "recording", streamingWindow: true });
  await fire("fade-overlay", 7);
  expect(rootClass()).not.toContain("is-fading");
});
test("only the newly committed tail animates, and a rewrite animates nothing", async () => {
  const arriving = () =>
    renderer.root
      .findAllByProps({ className: "transcript-arriving" })
      .map((node) => node.props.children as string);
  await fire("show-overlay", { state: "recording", streamingWindow: true });
  await fire("stream-text", { committed: "Hello there", tentative: "" });
  expect(arriving()).toEqual(["Hello there"]);
  await fire("stream-text", { committed: "Hello there, world", tentative: "" });
  expect(arriving()).toEqual([", world"]);
  // A tentative-only update must not restart the animation on settled words.
  await fire("stream-text", {
    committed: "Hello there, world",
    tentative: " and",
  });
  expect(arriving()).toEqual([", world"]);
  // A decoder revision replaces the line; animating it would read as a glitch.
  await fire("stream-text", {
    committed: "Completely revised.",
    tentative: "",
  });
  expect(arriving()).toEqual([]);
});

test("failed clipboard access offers retry without claiming success", async () => {
  await fire("show-overlay", { state: "recording", streamingWindow: true });
  await fire("stream-text", { committed: "Words to copy.", tentative: "" });
  copyResult = Promise.reject(new Error("Clipboard busy"));
  const button = renderer.root.findByType("button");
  await act(async () => {
    await button.props.onClick();
  });
  expect(button.props["aria-label"]).toBe("overlay.copyFailed");
  copyResult = Promise.resolve();
  await act(async () => {
    await button.props.onClick();
  });
  expect(button.props["aria-label"]).toBe("overlay.copied");
});

test("transcription and cleanup share an indeterminate bar, then finish on success", async () => {
  await fire("show-overlay", { state: "recording", streamingWindow: true });
  await fire("stream-text", {
    committed: "Keep these words visible.",
    tentative: "",
  });
  expect(renderer.root.findAllByProps({ role: "progressbar" })).toHaveLength(0);
  for (const state of ["transcribing", "processing"]) {
    await fire("show-overlay", { state, streamingWindow: true });
    const bar = renderer.root.findByProps({ role: "progressbar" });
    expect(bar.props["aria-valuenow"]).toBeUndefined();
    expect(bar.props["aria-label"]).toBe(`overlay.${state}`);
    expect(
      renderer.root
        .findAllByType("svg")
        .some((node) => node.props.className?.includes("audio-waveform")),
    ).toBe(false);
  }
  await fire("finish-overlay", { epoch: 9, text: "Keep these words visible." });
  expect(
    renderer.root.findByProps({ role: "progressbar" }).props["aria-valuenow"],
  ).toBe(100);
  expect(renderer.root.findByType("button")).toBeDefined();
});

test("a quick compact result completes during hide; cancellation never claims completion", async () => {
  await fire("show-overlay", { state: "transcribing", streamingWindow: false });
  await fire("hide-overlay");
  expect(
    renderer.root.findByProps({ role: "progressbar" }).props["aria-valuenow"],
  ).toBeUndefined();
  await fire("show-overlay", { state: "transcribing", streamingWindow: false });
  await fire("finish-overlay", { epoch: 10, text: "" });
  await fire("hide-overlay");
  expect(rootClass()).toContain("native-window-hidden");
  expect(
    renderer.root.findByProps({ role: "progressbar" }).props["aria-valuenow"],
  ).toBe(100);
  await fire("show-overlay", { state: "recording", streamingWindow: false });
  expect(renderer.root.findAllByProps({ role: "progressbar" })).toHaveLength(0);
});
