import { describe, expect, test } from "bun:test";
import type { ModelInfo } from "@/bindings";
import {
  compareCatalogOrder,
  filterLanguages,
  hasFilters,
  isOffered,
  matchesFilters,
  NO_FILTERS,
  RECOMMENDED_FOR_LANGUAGE,
  recommendedModels,
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

describe("recommendedModels", () => {
  const ids = (list: ModelInfo[]) => list.map((m) => m.id);
  const parakeet = model({
    id: "parakeet",
    is_recommended: true,
    recommended_rank: 1,
    supported_languages: ["en", "de"],
  });
  const whisperMedium = model({
    id: "whisper-medium",
    is_recommended: true,
    recommended_rank: 2,
    supported_languages: ["en", "ru", "ne"],
  });
  const canary = model({
    id: "canary",
    accuracy_score: 0.9,
    speed_score: 0.5,
    supported_languages: ["en", "ru"],
  });
  const fastSmall = model({
    id: "fast-small",
    accuracy_score: 0.5,
    speed_score: 0.9,
    supported_languages: ["ru"],
  });
  // Most accurate, but too slow to keep up: the balanced score puts it last.
  const huge = model({
    id: "huge",
    accuracy_score: 0.95,
    speed_score: 0.1,
    supported_languages: ["ru"],
  });
  const catalog = [huge, fastSmall, canary, whisperMedium, parakeet];

  test("all languages: the catalog's own picks, in catalog order", () => {
    expect(ids(recommendedModels(catalog, "all"))).toEqual([
      "parakeet",
      "whisper-medium",
    ]);
  });

  test("one language keeps only the picks that hear it", () => {
    const enPicks = [
      parakeet,
      whisperMedium,
      model({
        id: "nemotron",
        is_recommended: true,
        recommended_rank: 3,
        supported_languages: ["en"],
      }),
    ];
    // Three picks hear English, so nothing is topped up.
    expect(ids(recommendedModels([...enPicks, canary], "en"))).toEqual([
      "parakeet",
      "whisper-medium",
      "nemotron",
    ]);
    // Only Parakeet hears German, and no other model does.
    expect(ids(recommendedModels(catalog, "de"))).toEqual(["parakeet"]);
  });

  test(`tops up to ${RECOMMENDED_FOR_LANGUAGE} by accuracy and speed together`, () => {
    // canary 0.74, fast-small 0.66, huge 0.61: the most accurate model loses
    // to two that can keep up.
    expect(ids(recommendedModels(catalog, "ru"))).toEqual([
      "whisper-medium",
      "canary",
      "fast-small",
    ]);
  });

  test("a score tie falls back to catalog order", () => {
    const tied = [
      model({ id: "b", name: "Beta", supported_languages: ["ne"] }),
      model({ id: "a", name: "Alpha", supported_languages: ["ne"] }),
    ];
    expect(ids(recommendedModels(tied, "ne"))).toEqual(["a", "b"]);
  });

  test("a language only non-picks hear still gets a short list", () => {
    expect(ids(recommendedModels(catalog, "ne"))).toEqual(["whisper-medium"]);
    expect(
      ids(
        recommendedModels(
          [
            model({
              id: "x",
              supported_languages: ["ne"],
              accuracy_score: 0.4,
            }),
            model({
              id: "y",
              supported_languages: ["ne"],
              accuracy_score: 0.8,
            }),
          ],
          "ne",
        ),
      ),
    ).toEqual(["y", "x"]);
  });

  test("never recommends a legacy, retired, custom or local model", () => {
    const own = [
      model({ id: "legacy", is_legacy: true, accuracy_score: 1 }),
      model({ id: "retired", is_retired: true, accuracy_score: 1 }),
      model({ id: "custom", is_custom: true, accuracy_score: 1 }),
      model({ id: "local", local_path: "/models/x.gguf", accuracy_score: 1 }),
      model({
        id: "retired-pick",
        is_recommended: true,
        is_retired: true,
      }),
    ];
    expect(recommendedModels(own, "all")).toEqual([]);
    expect(recommendedModels(own, "en")).toEqual([]);
  });
});
