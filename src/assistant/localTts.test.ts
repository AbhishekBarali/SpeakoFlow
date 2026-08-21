import { describe, expect, test } from "bun:test";
import {
  IDLE_UNLOAD_HIDDEN_MS,
  IDLE_UNLOAD_VISIBLE_MS,
  idleUnloadDelayMs,
  isSpeechInFlight,
  localTtsActive,
  usesLocalTtsEngine,
} from "./localTts";

describe("localTtsActive", () => {
  test("a remote engine never mounts the local model", () => {
    // The regression this guards: spoken replies on, engine = Azure, and the
    // assistant panel still loaded ~310 MB of Kokoro weights (plus WebGPU
    // buffers) into a window that is never closed — for a model the backend
    // would never ask to speak.
    for (const engine of [
      "azure",
      "openai",
      "openrouter",
      "elevenlabs",
      "compatible",
    ]) {
      expect(localTtsActive(true, engine)).toBe(false);
    }
  });

  test("the local engine mounts only while spoken replies are on", () => {
    expect(localTtsActive(true, "kokoro")).toBe(true);
    expect(localTtsActive(false, "kokoro")).toBe(false);
  });

  test("settings with no engine recorded mean the local engine", () => {
    expect(usesLocalTtsEngine(undefined)).toBe(true);
    expect(usesLocalTtsEngine(null)).toBe(true);
    expect(localTtsActive(true, undefined)).toBe(true);
    expect(localTtsActive(false, undefined)).toBe(false);
  });
});

describe("isSpeechInFlight", () => {
  const idle = {
    streamOpen: false,
    synthDone: true,
    queued: 0,
    elementPlaying: false,
    nativePlaying: false,
  };

  test("nothing pending is idle, so the model may be released", () => {
    expect(isSpeechInFlight(idle)).toBe(false);
  });

  test("a drained queue mid-reply is still busy", () => {
    // A streaming reply empties the queue between sentences while the model is
    // synthesizing the next one; unloading there would cut the answer off.
    expect(isSpeechInFlight({ ...idle, streamOpen: true })).toBe(true);
    expect(isSpeechInFlight({ ...idle, synthDone: false })).toBe(true);
  });

  test("queued or playing audio is busy", () => {
    expect(isSpeechInFlight({ ...idle, queued: 1 })).toBe(true);
    expect(isSpeechInFlight({ ...idle, elementPlaying: true })).toBe(true);
    expect(isSpeechInFlight({ ...idle, nativePlaying: true })).toBe(true);
  });
});

describe("idleUnloadDelayMs", () => {
  test("a dismissed panel gives its memory back sooner than an open one", () => {
    expect(idleUnloadDelayMs(false)).toBe(IDLE_UNLOAD_HIDDEN_MS);
    expect(idleUnloadDelayMs(true)).toBe(IDLE_UNLOAD_VISIBLE_MS);
    expect(IDLE_UNLOAD_HIDDEN_MS).toBeLessThan(IDLE_UNLOAD_VISIBLE_MS);
  });

  test("both windows are finite, so an idle model is always released", () => {
    expect(Number.isFinite(IDLE_UNLOAD_HIDDEN_MS)).toBe(true);
    expect(Number.isFinite(IDLE_UNLOAD_VISIBLE_MS)).toBe(true);
    expect(IDLE_UNLOAD_HIDDEN_MS).toBeGreaterThan(0);
  });
});
