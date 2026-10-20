import { describe, expect, test } from "bun:test";
import type { ModelInfo } from "@/bindings";
import {
  compareCatalogOrder,
  filterLanguages,
  hasFilters,
  isOffered,
  matchesFilters,
  NO_FILTERS,
} from "./speechCatalog";

const model = (overrides: Partial<ModelInfo> & { id: string }): ModelInfo => ({
  name: overrides.id,
  description: "",
  filename: `${overrides.id}.gguf`,
  url: null,
  sha256: null,
  size_mb: 100,
  is_downloaded: false,
  is_downloading: false,
  partial_size: 0,
  is_directory: false,
  engine_type: "TranscribeCpp",
  accuracy_score: 0.5,
  speed_score: 0.5,
  supports_translation: false,
  supports_streaming: false,
  is_recommended: false,
  recommended_rank: null,
  supported_languages: ["en"],
  supports_language_selection: false,
  is_custom: false,
  local_path: null,
  local_folder: null,
  ...overrides,
});

describe("compareCatalogOrder", () => {
  test("rank, then recommended, then accuracy, speed and name", () => {
    const list = [
      model({ id: "g", name: "Zulu", speed_score: 0.1 }),
      model({ id: "d", accuracy_score: 0.9 }),
      model({ id: "f", name: "Alpha", speed_score: 0.1 }),
      model({ id: "c", is_recommended: true }),
      model({ id: "e", speed_score: 0.9 }),
      model({ id: "b", recommended_rank: 2 }),
      model({ id: "a", is_recommended: true, recommended_rank: 1 }),
    ];
    expect([...list].sort(compareCatalogOrder).map((m) => m.id)).toEqual([
      "a",
      "b",
      "c",
      "d",
      "e",
      "f",
      "g",
    ]);
  });
});

describe("isOffered", () => {
  test("an older model only while it is on disk", () => {
    expect(isOffered(model({ id: "medium", is_legacy: true }), false)).toBe(
      false,
    );
    expect(isOffered(model({ id: "medium", is_legacy: true }), true)).toBe(
      true,
    );
    expect(isOffered(model({ id: "whisper-medium-gguf" }), false)).toBe(true);
  });
});

describe("matchesFilters", () => {
  const nemotron = model({
    id: "nemotron",
    supports_streaming: true,
    supported_languages: ["en", "nb", "zh"],
  });
  const whisper = model({
    id: "whisper",
    supports_translation: true,
    supported_languages: ["en", "zh-Hans", "zh-Hant", "no", "yue"],
  });
  const retired = model({
    id: "moonshine-tiny-streaming-en",
    supports_streaming: true,
    is_retired: true,
  });

  test("no filters keep everything", () => {
    expect(hasFilters(NO_FILTERS)).toBe(false);
    for (const m of [nemotron, whisper, retired]) {
      expect(matchesFilters(m, NO_FILTERS)).toBe(true);
    }
  });

  test("Chinese finds every Chinese model, Norwegian finds nb", () => {
    const chinese = { ...NO_FILTERS, language: "zh" };
    expect(matchesFilters(nemotron, chinese)).toBe(true);
    expect(matchesFilters(whisper, chinese)).toBe(true);
    const norwegian = { ...NO_FILTERS, language: "no" };
    expect(matchesFilters(nemotron, norwegian)).toBe(true);
    expect(matchesFilters(whisper, norwegian)).toBe(true);
    expect(
      matchesFilters(model({ id: "en" }), { ...NO_FILTERS, language: "zh" }),
    ).toBe(false);
  });

  test("streaming and translate leave retired models out", () => {
    const streaming = { ...NO_FILTERS, streaming: true };
    expect(matchesFilters(nemotron, streaming)).toBe(true);
    expect(matchesFilters(whisper, streaming)).toBe(false);
    expect(matchesFilters(retired, streaming)).toBe(false);
    const translate = { ...NO_FILTERS, translate: true };
    expect(matchesFilters(whisper, translate)).toBe(true);
    expect(matchesFilters(nemotron, translate)).toBe(false);
    expect(hasFilters(translate)).toBe(true);
  });

  test("filters combine", () => {
    const both = { language: "zh", streaming: true, translate: false };
    expect(matchesFilters(nemotron, both)).toBe(true);
    expect(matchesFilters(whisper, both)).toBe(false);
  });
});

describe("filterLanguages", () => {
  test("lists each language once", () => {
    expect(
      filterLanguages([
        model({ id: "a", supported_languages: ["en", "nb", "zh"] }),
        model({
          id: "b",
          supported_languages: ["en-US", "no", "zh-Hans", "zh-Hant", "yue"],
        }),
      ]),
    ).toEqual(["en", "no", "zh", "yue"]);
  });
});
