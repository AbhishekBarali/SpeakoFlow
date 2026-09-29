import { describe, expect, test } from "bun:test";
import {
  DEFAULT_NATIVE_VOICE,
  NATIVE_ENGINES,
  effectiveNativeVoice,
  isNativeEngine,
  kittenSizeFor,
  nativePackId,
  nativeVoices,
} from "./nativeVoices";

/** These mirror `native_tts.rs`; a drift here speaks with the wrong pack or
 *  the wrong speaker, which no error would ever reveal. */
describe("native voices", () => {
  test("engines resolve to the pack the backend loads", () => {
    expect(nativePackId("kitten", "")).toBe("kitten-nano-0.8");
    expect(nativePackId("kitten", "kitten-mini-0.8")).toBe("kitten-mini-0.8");
    expect(nativePackId("kitten", "micro")).toBe("kitten-micro-0.8");
    expect(nativePackId("kitten", "gpt-4o-mini-tts")).toBe("kitten-nano-0.8");
    expect(nativePackId("pocket", "")).toBe("pocket-tts");
    expect(nativePackId("supertonic", "")).toBe("supertonic-3-int8");
    expect(nativePackId("kokoro", "")).toBeNull();
    expect(nativePackId("openai", "tts-1")).toBeNull();
  });

  test("kitten sizes parse leniently", () => {
    expect(kittenSizeFor(undefined)).toBe("nano");
    expect(kittenSizeFor(" Mini ")).toBe("mini");
    expect(kittenSizeFor("kitten-micro-0.8")).toBe("micro");
  });

  test("voice counts match the backend", () => {
    expect(nativeVoices("kitten")).toHaveLength(8);
    expect(nativeVoices("pocket")).toHaveLength(14);
    expect(nativeVoices("supertonic").map((v) => v.id)).toEqual([
      "F1",
      "F2",
      "F3",
      "F4",
      "F5",
      "M1",
      "M2",
      "M3",
      "M4",
      "M5",
    ]);
  });

  test("an unknown or foreign voice falls back to the engine default", () => {
    for (const engine of NATIVE_ENGINES) {
      expect(isNativeEngine(engine)).toBe(true);
      expect(effectiveNativeVoice(engine, "af_heart")).toBe(
        DEFAULT_NATIVE_VOICE[engine],
      );
      const ids = nativeVoices(engine).map((v) => v.id);
      expect(ids).toContain(DEFAULT_NATIVE_VOICE[engine]);
      expect(new Set(ids).size).toBe(ids.length);
    }
    expect(effectiveNativeVoice("pocket", "george")).toBe("George");
    // Dropped for their recordings; a stored one falls back, as in Rust.
    expect(effectiveNativeVoice("pocket", "Alba")).toBe("Mary");
    expect(isNativeEngine("kokoro")).toBe(false);
  });
});
