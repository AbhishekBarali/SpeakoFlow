import { describe, expect, test } from "bun:test";
import {
  appLanguageCode,
  canonicalLanguageCode,
  effectiveLanguage,
  languagesMatch,
  modelSupportsLanguage,
  uniqueLanguages,
} from "./languageCodes";

// The same cases as `src-tauri/src/managers/language.rs`, so the picker and
// the transcription path agree.

describe("canonicalLanguageCode", () => {
  test("drops region and script, folds the two aliases", () => {
    expect(canonicalLanguageCode("en")).toBe("en");
    expect(canonicalLanguageCode("en-US")).toBe("en");
    expect(canonicalLanguageCode("EN_gb")).toBe("en");
    expect(canonicalLanguageCode("zh-Hans")).toBe("zh");
    expect(canonicalLanguageCode("zh-Hant")).toBe("zh");
    expect(canonicalLanguageCode("nb")).toBe("no");
    expect(canonicalLanguageCode("no")).toBe("no");
    expect(canonicalLanguageCode("nn")).toBe("nn");
    expect(canonicalLanguageCode("fil")).toBe("tl");
    expect(canonicalLanguageCode("tl")).toBe("tl");
    expect(canonicalLanguageCode("yue")).toBe("yue");
    expect(canonicalLanguageCode("")).toBe("");
  });
});

describe("languagesMatch", () => {
  test("aliases and script variants match, Cantonese and Nynorsk do not", () => {
    expect(languagesMatch("nb", "no")).toBe(true);
    expect(languagesMatch("fil", "tl")).toBe(true);
    expect(languagesMatch("zh-Hans", "zh")).toBe(true);
    expect(languagesMatch("zh-Hant", "zh")).toBe(true);
    expect(languagesMatch("zh-Hans", "zh-Hant")).toBe(true);
    expect(languagesMatch("yue", "zh")).toBe(false);
    expect(languagesMatch("nn", "no")).toBe(false);
    expect(languagesMatch("", "")).toBe(false);
  });

  test("a model that lists zh offers both Chinese scripts", () => {
    expect(modelSupportsLanguage(["en", "zh"], "zh-Hans")).toBe(true);
    expect(modelSupportsLanguage(["en", "zh"], "zh-Hant")).toBe(true);
    expect(modelSupportsLanguage(["en", "zh"], "yue")).toBe(false);
    expect(modelSupportsLanguage(["en", "nb"], "no")).toBe(true);
  });
});

describe("appLanguageCode", () => {
  test("writes a model's code the app's way", () => {
    expect(appLanguageCode("zh")).toBe("zh-Hans");
    expect(appLanguageCode("zh-Hant")).toBe("zh-Hant");
    expect(appLanguageCode("zh-Hans")).toBe("zh-Hans");
    expect(appLanguageCode("nb")).toBe("no");
    expect(appLanguageCode("fil")).toBe("tl");
    expect(appLanguageCode("en-US")).toBe("en");
    expect(appLanguageCode("yue")).toBe("yue");
  });
});

describe("uniqueLanguages", () => {
  test("each language once, in canonical form", () => {
    expect(
      uniqueLanguages(["en-US", "en-GB", "nb", "no", "zh", "zh-Hans", "yue"]),
    ).toEqual(["en", "no", "zh", "yue"]);
    expect(uniqueLanguages(["", " "])).toEqual([]);
  });
});

describe("effectiveLanguage", () => {
  const whisper = ["en", "zh", "zh-Hans", "zh-Hant", "de", "no", "tl", "yue"];
  const nemotron = ["en", "de", "nb", "zh"];
  const english = ["en"];
  const breeze = ["zh", "en"];
  const chineseOnly = ["zh"];
  const cantoneseOnly = ["yue"];
  const locales = ["en-US", "fil-PH"];

  test("keeps the intent when the model lists it, in the app's spelling", () => {
    expect(effectiveLanguage("de", whisper, true)).toBe("de");
    expect(effectiveLanguage("zh-Hans", nemotron, false)).toBe("zh-Hans");
    expect(effectiveLanguage("zh-Hant", breeze, false)).toBe("zh-Hant");
    expect(effectiveLanguage("no", nemotron, false)).toBe("no");
    expect(effectiveLanguage("tl", locales, false)).toBe("tl");
    expect(effectiveLanguage("en", locales, false)).toBe("en");
  });

  test("Cantonese is not Chinese", () => {
    expect(effectiveLanguage("yue", nemotron, true)).toBe("auto");
    expect(effectiveLanguage("zh-Hans", cantoneseOnly, false)).toBe("yue");
  });

  test("falls back to auto, then English, then the first language", () => {
    expect(effectiveLanguage("ja", whisper, true)).toBe("auto");
    expect(effectiveLanguage("auto", whisper, true)).toBe("auto");
    expect(effectiveLanguage("ja", nemotron, false)).toBe("en");
    expect(effectiveLanguage("auto", english, false)).toBe("en");
    expect(effectiveLanguage("auto", locales, false)).toBe("en");
    expect(effectiveLanguage("auto", chineseOnly, false)).toBe("zh-Hans");
    expect(effectiveLanguage("de", ["nb", "sv"], false)).toBe("no");
  });

  test("a model without a list takes the intent as it is", () => {
    expect(effectiveLanguage("ja", [], false)).toBe("ja");
    expect(effectiveLanguage("auto", [], false)).toBe("auto");
  });
});
