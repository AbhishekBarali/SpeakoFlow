import { describe, expect, test } from "bun:test";
import { showsChineseScript } from "./chineseScriptRule";

// The same cases as `chinese_script::tests` in the Rust transcription path.
describe("showsChineseScript", () => {
  const whisper = ["en", "zh", "yue", "ja"];

  test("a script chosen as the language needs no second choice", () => {
    expect(showsChineseScript("zh-Hans", whisper)).toBe(false);
    expect(showsChineseScript("zh-Hant", undefined)).toBe(false);
  });

  test("plain Chinese, Cantonese and auto on a model that writes Chinese", () => {
    expect(showsChineseScript("auto", whisper)).toBe(true);
    expect(showsChineseScript("zh", whisper)).toBe(true);
    expect(showsChineseScript("yue", whisper)).toBe(true);
    expect(showsChineseScript("auto", ["en", "yue"])).toBe(true);
  });

  test("an intent the model does not have, detected instead", () => {
    expect(showsChineseScript("ko", ["en", "zh"], true)).toBe(true);
  });

  test("never for a model that cannot write Chinese, or another language", () => {
    expect(showsChineseScript("auto", ["en", "ja"])).toBe(false);
    expect(showsChineseScript("ja", whisper)).toBe(false);
    expect(showsChineseScript("de", [...whisper, "de"])).toBe(false);
    // Chinese asked of an English-only model is English.
    expect(showsChineseScript("zh", ["en"], false)).toBe(false);
  });

  test("a cloud provider can write anything", () => {
    expect(showsChineseScript("auto", undefined)).toBe(true);
    expect(showsChineseScript("auto", [])).toBe(true);
    expect(showsChineseScript("yue", undefined)).toBe(true);
    expect(showsChineseScript("en", undefined)).toBe(false);
  });
});
