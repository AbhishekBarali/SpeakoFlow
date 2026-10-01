import { describe, expect, test } from "bun:test";
import {
  languageLabel,
  languageMatches,
  localizedLanguages,
} from "./languages";

describe("languageLabel", () => {
  test("English UI keeps the catalog's English names", () => {
    expect(languageLabel("de", "en")).toBe("German");
    expect(languageLabel("zh-Hans", "en")).toBe("Simplified Chinese");
  });

  test("other UI languages get the name in their own language", () => {
    expect(languageLabel("de", "de")).toBe("Deutsch");
    expect(languageLabel("ja", "ja")).toBe("日本語");
  });

  test("a lowercase language name starts with a capital in a picker", () => {
    // French writes language names in lowercase ("allemand").
    expect(languageLabel("de", "fr")).toBe("Allemand");
  });

  test("Whisper's legacy Javanese code still resolves", () => {
    expect(languageLabel("jw", "de")).toBe("Javanisch");
  });

  test("auto and unknown codes fall back to the English label", () => {
    expect(languageLabel("auto", "de")).toBe("Auto Detect");
    expect(languageLabel("not-a-code", "de")).toBe("not-a-code");
  });
});

describe("languageMatches", () => {
  test("search finds a language by its localized or English name", () => {
    const german = localizedLanguages("de").find((l) => l.value === "de")!;
    expect(languageMatches(german, "deut")).toBe(true);
    expect(languageMatches(german, "germ")).toBe(true);
    expect(languageMatches(german, "fran")).toBe(false);
    expect(languageMatches(german, "  ")).toBe(true);
  });
});
