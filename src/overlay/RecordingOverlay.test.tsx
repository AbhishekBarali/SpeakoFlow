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

type NodeMock = (element: { props: Record<string, unknown> }) => unknown;
const plainNode: NodeMock = () => ({
  matches: () => false,
  scrollTop: 0,
  scrollHeight: 0,
});
let renderer: ReactTestRenderer;
const mount = async (createNodeMock: NodeMock = plainNode) => {
  await act(async () => {
    renderer = create(<RecordingOverlay />, { createNodeMock });
  });
};
const fire = async (name: string, payload: unknown = null) => {
  await act(async () => {
    events.get(name)?.({ payload });
  });
};
const rootClass = () =>
  renderer.root.findByProps({ dir: "ltr" }).props.className as string;
/** The body, whichever of its two classes it has right now. */
const cardBody = () =>
  renderer.root.find(
    (node) =>
      typeof node.type === "string" &&
      String(node.props.className ?? "").startsWith("card-body"),
  );
/** The transcript exactly as drawn: every chunk, then the tentative tail. */
const shownText = () =>
  renderer.root
    .findByProps({ className: "transcript-line" })
    .findAllByType("span")
    .map((span) =>
      typeof span.props.children === "string" ? span.props.children : "",
    )
    .join("");
const press = async () => {
  await act(async () => {
    cardBody().props.onPointerDown({ button: 0, clientX: 0 });
  });
};
const release = async () => {
  await act(async () => {
    cardBody().props.onPointerUp();
  });
};
/** Stand in for the webview's selection while a test runs. */
let selected = "";
const withSelection = async (run: () => Promise<void>) => {
  const original = (globalThis as { window?: unknown }).window;
  (globalThis as { window?: unknown }).window = {
    getSelection: () => ({ toString: () => selected }),
  };
  try {
    await run();
  } finally {
    (globalThis as { window?: unknown }).window = original;
    selected = "";
  }
};

beforeEach(async () => {
  calls.length = 0;
  copyResult = Promise.resolve();
  events.clear();
  await mount();
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

/** The label a `downloading` pill is showing right now. */
const downloadLabel = () =>
  renderer.root.findByProps({ className: "pill-label" }).props
    .children as string;

test("a still-downloading speech model follows only its own progress", async () => {
  await fire("show-overlay", {
    state: "downloading",
    streamingWindow: false,
    download: "parakeet",
  });
  // Nothing has arrived yet: no number is claimed.
  expect(downloadLabel()).toBe("overlay.downloading.title");
  expect(
    renderer.root.findAll(
      (node) =>
        node.type === "svg" &&
        String(node.props.className).includes("is-waiting"),
    ),
  ).toHaveLength(1);

  // Another model's download is not this pill's business.
  await fire("model-download-progress", {
    model_id: "gemma-4-e2b",
    percentage: 90,
  });
  expect(downloadLabel()).toBe("overlay.downloading.title");

  await fire("model-download-progress", {
    model_id: "parakeet",
    percentage: 41.6,
  });
  expect(downloadLabel()).toBe("overlay.downloading.percent");
  expect(
    renderer.root.findAll(
      (node) =>
        node.type === "svg" &&
        String(node.props.className).includes("is-waiting"),
    ),
  ).toHaveLength(0);
  // The pill is status only: nothing on it takes a click.
  expect(renderer.root.findAllByType("button")).toHaveLength(0);

  // A hide forgets the download, so a late event cannot revive a number on
  // the next pill.
  await fire("hide-overlay");
  await fire("show-overlay", {
    state: "downloading",
    streamingWindow: false,
    download: "parakeet",
  });
  expect(downloadLabel()).toBe("overlay.downloading.title");
});

test("a hop to another display fades out, then back in, and is never left hidden", async () => {
  await fire("show-overlay", { state: "recording", streamingWindow: false });
  await fire("overlay-hop", "out");
  expect(rootClass()).toContain("is-hopping");
  await fire("overlay-hop", "in");
  expect(rootClass()).not.toContain("is-hopping");

  // Interrupted by the next state: the show placed the window itself.
  await fire("overlay-hop", "out");
  await fire("show-overlay", { state: "transcribing", streamingWindow: false });
  expect(rootClass()).not.toContain("is-hopping");

  // Interrupted by a hide.
  await fire("overlay-hop", "out");
  await fire("hide-overlay");
  expect(rootClass()).not.toContain("is-hopping");

  // A hidden overlay ignores a late hop entirely.
  await fire("overlay-hop", "out");
  expect(rootClass()).not.toContain("is-hopping");
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
  // A copy from a window that never takes the keyboard says so in words.
  expect(
    renderer.root.findByProps({ className: "card-copied" }).props.children,
  ).toBe("overlay.copied");
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
  expect(shownText()).toBe("Hello there, world and");
  // A decoder revision replaces the line; animating it would read as a glitch.
  await fire("stream-text", {
    committed: "Completely revised.",
    tentative: "",
  });
  expect(arriving()).toEqual([]);
  expect(shownText()).toBe("Completely revised.");
});

test("committed words are appended as new runs, never rewritten in place", async () => {
  // Rewriting a text node collapses any selection inside it, so words already
  // on screen must keep their node while new ones arrive.
  await fire("show-overlay", { state: "recording", streamingWindow: true });
  await fire("stream-text", { committed: "One.", tentative: "" });
  const first = renderer.root.findByProps({ children: "One." });
  await fire("stream-text", { committed: "One. Two.", tentative: "" });
  await fire("stream-text", { committed: "One. Two. Three.", tentative: "" });
  expect(renderer.root.findByProps({ children: "One." })).toBe(first);
  expect(shownText()).toBe("One. Two. Three.");
});

test("the live card offers copy and selection only where it takes the pointer", async () => {
  // Click-through (an X11 window mid-recording): a button or a selectable line
  // here would be a control the window passes straight to the app underneath.
  await fire("show-overlay", { state: "recording", streamingWindow: true });
  await fire("stream-text", { committed: "Words so far.", tentative: "" });
  expect(renderer.root.findAllByType("button")).toHaveLength(0);
  expect(cardBody().props.className).toBe("card-body");
  // Where the card cannot take focus it is live from the first word, through
  // every working state.
  for (const state of ["recording", "transcribing", "processing"]) {
    await fire("show-overlay", {
      state,
      streamingWindow: true,
      interactive: true,
    });
    if (state === "recording")
      await fire("stream-text", { committed: "Words so far.", tentative: "" });
    expect(renderer.root.findAllByType("button")).toHaveLength(1);
    expect(cardBody().props.className).toBe("card-body is-selectable");
  }
});

test("failed clipboard access offers retry without claiming success", async () => {
  await fire("show-overlay", {
    state: "recording",
    streamingWindow: true,
    interactive: true,
  });
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

test("transcription and cleanup settle the same bars into a working ripple, then a check replaces them", async () => {
  await fire("show-overlay", { state: "recording", streamingWindow: true });
  await fire("stream-text", {
    committed: "Keep these words visible.",
    tentative: "",
  });
  expect(renderer.root.findAllByProps({ role: "progressbar" })).toHaveLength(0);
  // Five bars while listening, not a barcode of fourteen.
  expect(renderer.root.findAllByType("line")).toHaveLength(5);
  const listening = renderer.root
    .findAllByType("svg")
    .find((node) => node.props.className?.includes("audio-waveform"));
  expect(listening?.props.className).toContain("reactive");
  for (const state of ["transcribing", "processing"]) {
    await fire("show-overlay", { state, streamingWindow: true });
    const working = renderer.root.findByProps({ role: "progressbar" });
    expect(working.props["aria-valuenow"]).toBeUndefined();
    expect(working.props["aria-label"]).toBe(`overlay.${state}`);
    // The same five bars, now in working mode — no dots, no sweeping bar.
    const waves = renderer.root
      .findAllByType("svg")
      .filter((node) => node.props.className?.includes("audio-waveform"));
    expect(waves).toHaveLength(1);
    expect(waves[0].props.className).toContain("working");
    expect(renderer.root.findAllByType("line")).toHaveLength(5);
    expect(
      renderer.root.findAllByProps({ className: "overlay-working-dot" }),
    ).toHaveLength(0);
  }
  await fire("finish-overlay", { epoch: 9, text: "Keep these words visible." });
  expect(renderer.root.findAllByProps({ role: "progressbar" })).toHaveLength(0);
  const mark = renderer.root.findByProps({ className: "completion-mark" });
  expect(mark.props["aria-label"]).toBe("overlay.done");
  // The check stands alone; a visible "Done" word would only repeat it.
  expect(
    renderer.root.findAllByProps({ className: "card-label" }),
  ).toHaveLength(0);
  expect(renderer.root.findAllByType("circle")).toHaveLength(0);
  expect(renderer.root.findByType("button")).toBeDefined();
  // The same words as the live text: nothing to fade in.
  expect(
    renderer.root.findAllByProps({ className: "card-text is-revealed" }),
  ).toHaveLength(0);
});

test("a completion straight from recording shows the same mark", async () => {
  // Streaming engines can finish without ever showing a transcribing state.
  await fire("show-overlay", { state: "recording", streamingWindow: true });
  await fire("stream-text", { committed: "Short one.", tentative: "" });
  await fire("finish-overlay", { epoch: 11, text: "Short one." });
  expect(
    renderer.root.findAllByProps({ className: "completion-mark" }),
  ).toHaveLength(1);
  expect(
    renderer.root
      .findAllByType("svg")
      .some((node) => node.props.className?.includes("audio-waveform")),
  ).toBe(false);
});

test("a quick compact result completes during hide; cancellation never claims completion", async () => {
  const marks = () =>
    renderer.root.findAllByProps({ className: "completion-mark" });
  await fire("show-overlay", { state: "transcribing", streamingWindow: false });
  await fire("hide-overlay");
  expect(marks()).toHaveLength(0);
  expect(renderer.root.findAllByProps({ role: "progressbar" })).toHaveLength(1);
  await fire("show-overlay", { state: "transcribing", streamingWindow: false });
  await fire("finish-overlay", { epoch: 10, text: "" });
  await fire("hide-overlay");
  expect(rootClass()).toContain("native-window-hidden");
  expect(marks()).toHaveLength(1);
  expect(marks()[0].props["aria-label"]).toBe("overlay.done");
  await fire("show-overlay", { state: "recording", streamingWindow: false });
  expect(marks()).toHaveLength(0);
  expect(renderer.root.findAllByProps({ role: "progressbar" })).toHaveLength(0);
});

test("releasing a selection copies it, but only where the card takes the pointer", async () => {
  await withSelection(async () => {
    await fire("show-overlay", { state: "recording", streamingWindow: true });
    await fire("stream-text", { committed: "Still speaking.", tentative: "" });
    // Click-through: a press never reaches the card, so nothing is copied.
    selected = "Still";
    await press();
    await release();
    expect(calls).toHaveLength(0);
    // A finished card takes the pointer everywhere.
    await fire("finish-overlay", { epoch: 12, text: "Pick these words out." });
    selected = "";
    await press();
    await release();
    expect(calls).toHaveLength(0);
    selected = " these words ";
    await press();
    await release();
    expect(calls).toEqual([
      { command: "copy_overlay_transcript", args: { text: "these words" } },
    ]);
    expect(renderer.root.findByType("button").props["aria-label"]).toBe(
      "overlay.copied",
    );
  });
});

test("a drag holds live words still, copies on release, then catches up", async () => {
  await withSelection(async () => {
    await fire("show-overlay", {
      state: "recording",
      streamingWindow: true,
      interactive: true,
    });
    await fire("stream-text", { committed: "First words.", tentative: "" });
    await press();
    await fire("stream-text", {
      committed: "First words. More arrived.",
      tentative: " still",
    });
    // Nothing moves under the pointer mid-selection.
    expect(shownText()).toBe("First words.");
    selected = " First ";
    await release();
    expect(calls).toEqual([
      { command: "copy_overlay_transcript", args: { text: "First" } },
    ]);
    expect(shownText()).toBe("First words. More arrived. still");
  });
});

test("the final text waits for the end of a drag, then fades in once", async () => {
  await withSelection(async () => {
    await fire("show-overlay", {
      state: "recording",
      streamingWindow: true,
      interactive: true,
    });
    await fire("stream-text", { committed: "um raw words", tentative: "" });
    await press();
    await fire("finish-overlay", { epoch: 20, text: "Clean final words." });
    expect(shownText()).toBe("um raw words");
    await release();
    expect(shownText()).toBe("Clean final words.");
    expect(
      renderer.root.findAllByProps({ className: "card-text is-revealed" }),
    ).toHaveLength(1);
    // A late live update cannot paint the raw transcript back over it.
    await fire("stream-text", {
      committed: "um raw words late",
      tentative: "",
    });
    expect(shownText()).toBe("Clean final words.");
  });
});

test("a new recording drops a drag the last one left behind", async () => {
  await fire("show-overlay", {
    state: "recording",
    streamingWindow: true,
    interactive: true,
  });
  await fire("stream-text", { committed: "Old words.", tentative: "" });
  await press();
  await fire("show-overlay", {
    state: "recording",
    streamingWindow: true,
    interactive: true,
  });
  await fire("stream-text", { committed: "New words.", tentative: "" });
  expect(shownText()).toBe("New words.");
});

test("the transcript follows new lines until the user scrolls back, then resumes at the end", async () => {
  act(() => renderer.unmount());
  const scrolls: { top: number; behavior: string }[] = [];
  const body = {
    scrollTop: 0,
    scrollHeight: 66,
    clientHeight: 66,
    scrollTo({ top, behavior }: { top: number; behavior: string }) {
      scrolls.push({ top, behavior });
      body.scrollTop = top;
    },
  };
  await mount((element) =>
    String(element.props.className ?? "").startsWith("card-body")
      ? body
      : { matches: () => false },
  );
  // What the browser reports after any scroll, programmatic or not.
  const scrolled = async (top: number) => {
    body.scrollTop = top;
    await act(async () => {
      cardBody().props.onScroll();
    });
  };
  await fire("show-overlay", {
    state: "recording",
    streamingWindow: true,
    interactive: true,
  });
  body.scrollHeight = 88;
  await fire("stream-text", {
    committed: "Four lines of words.",
    tentative: "",
  });
  expect(scrolls.at(-1)).toEqual({ top: 22, behavior: "smooth" });
  await scrolled(22);
  // Past the top edge: that edge fades instead of cutting a line in half.
  expect(cardBody().props["data-above"]).toBe("");
  expect(cardBody().props["data-below"]).toBeUndefined();

  // The user scrolls back to reread. New words must not yank them down.
  await scrolled(0);
  expect(cardBody().props["data-below"]).toBe("");
  const heldAt = scrolls.length;
  body.scrollHeight = 110;
  await fire("stream-text", {
    committed: "Four lines of words. And a fifth.",
    tentative: "",
  });
  expect(scrolls).toHaveLength(heldAt);

  // Back at the newest words, the card follows them again.
  await scrolled(44);
  expect(cardBody().props["data-below"]).toBeUndefined();
  body.scrollHeight = 132;
  await fire("stream-text", {
    committed: "Four lines of words. And a fifth. And a sixth.",
    tentative: "",
  });
  expect(scrolls.at(-1)).toEqual({ top: 66, behavior: "smooth" });

  // The final text is already where it belongs and fades in: no glide.
  body.scrollHeight = 44;
  await fire("finish-overlay", { epoch: 30, text: "Two tidy lines." });
  expect(scrolls.at(-1)).toEqual({ top: 0, behavior: "instant" });
});

/** Every text node under `node`, joined, for asserting what a pill says. */
const textOf = (node: ReturnType<typeof renderer.root.findByType>): string =>
  node.children
    .map((child) => (typeof child === "string" ? child : textOf(child)))
    .join("");

test("a dismissed dictation offers Undo, which sends back the pill's own epoch once", async () => {
  await fire("show-overlay", {
    state: "dismissed",
    streamingWindow: false,
    interactive: true,
    epoch: 12,
  });
  const pill = renderer.root.findByProps({ role: "group" });
  expect(textOf(pill)).toContain("overlay.recovery.dismissed");
  const buttons = renderer.root.findAllByType("button");
  expect(buttons).toHaveLength(1);
  expect(textOf(buttons[0])).toContain("overlay.recovery.undo");

  await act(async () => {
    buttons[0].props.onClick();
  });
  expect(calls).toEqual([
    { command: "recover_dictation", args: { epoch: 12 } },
  ]);
  // A second click while the first is being answered does nothing.
  expect(renderer.root.findByType("button").props.disabled).toBe(true);
  await act(async () => {
    renderer.root.findByType("button").props.onClick();
  });
  expect(calls).toHaveLength(1);

  // The working state that follows is an ordinary pill again.
  await fire("show-overlay", {
    state: "transcribing",
    streamingWindow: false,
    epoch: 13,
  });
  expect(renderer.root.findAllByType("button")).toHaveLength(0);
});

test("a failure explains itself and offers to try again, even with the live card on", async () => {
  await fire("show-overlay", {
    state: "failed",
    // The backend never sends the card for a recovery pill; the webview
    // must not draw one if it did.
    streamingWindow: true,
    interactive: true,
    notice: "cloudSttFailed",
    epoch: 4,
  });
  const pill = renderer.root.findByProps({ role: "group" });
  expect(String(pill.props.className)).toContain("has-notice");
  expect(String(pill.props.className)).not.toContain("overlay-card");
  expect(textOf(pill)).toContain("overlay.notices.cloudSttFailed");
  expect(textOf(renderer.root.findByType("button"))).toContain(
    "overlay.recovery.retry",
  );
});

test("the recovery pill fades with its own epoch, and a hide clears it", async () => {
  await fire("show-overlay", {
    state: "dismissed",
    streamingWindow: false,
    interactive: true,
    epoch: 20,
  });
  await fire("fade-overlay", 19);
  expect(rootClass()).not.toContain("is-fading");
  await fire("fade-overlay", 20);
  expect(rootClass()).toContain("is-fading");
  await fire("restore-overlay", 20);
  expect(rootClass()).not.toContain("is-fading");

  await fire("hide-overlay");
  expect(rootClass()).toContain("native-window-hidden");
  // A late click on a pill that has gone must not reach the backend.
  const buttons = renderer.root.findAllByType("button");
  if (buttons.length > 0) {
    await act(async () => {
      buttons[0].props.onClick();
    });
  }
  expect(calls).toHaveLength(0);
});
