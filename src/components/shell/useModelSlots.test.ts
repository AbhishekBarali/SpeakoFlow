import { describe, expect, test } from "bun:test";
import type { TFunction } from "i18next";
import type { AppSettings, ModelInfo } from "@/bindings";
import { summarizeStt } from "./useModelSlots";
import { isAutoSelectable } from "./useSttStatus";

/** Keys as themselves, except where a fallback is given (model names). */
const t = ((key: string, options?: { defaultValue?: string }) =>
  options?.defaultValue ?? key) as unknown as TFunction;

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

const local = { stt_engine_mode: "local" } as AppSettings;
const cloud = {
  stt_engine_mode: "cloud",
  cloud_stt_provider_id: "elevenlabs",
  cloud_stt_models: { elevenlabs: "scribe_v2" },
} as unknown as AppSettings;

const retired = model({
  id: "parakeet-tdt-0.6b-v3",
  name: "Parakeet V3",
  engine_type: "Parakeet",
  is_downloaded: true,
  is_legacy: true,
  is_retired: true,
  replacement_id: "parakeet-tdt-0.6b-v3-gguf",
});

describe("summarizeStt", () => {
  test("a retired selection on disk needs an update, never reads as working", () => {
    const summary = summarizeStt(local, [retired], retired.id, [], {}, t);
    expect(summary.ready).toBe(false);
    expect(summary.issue).toBe("needs_update");
    expect(summary.modelLabel).toBe("Parakeet V3");
    expect(summary.localModel?.id).toBe(retired.id);
  });

  test("a retired selection whose files are gone still needs an update", () => {
    const gone = { ...retired, is_downloaded: false };
    expect(summarizeStt(local, [gone], gone.id, [], {}, t).issue).toBe(
      "needs_update",
    );
  });

  test("a downloaded model that runs is ready", () => {
    const parakeet = model({ id: "parakeet-unified", is_downloaded: true });
    const summary = summarizeStt(local, [parakeet], parakeet.id, [], {}, t);
    expect(summary.ready).toBe(true);
    expect(summary.issue).toBeNull();
  });

  test("an older Whisper .bin still runs, so it is not an update", () => {
    const bin = model({
      id: "medium",
      engine_type: "Whisper",
      is_downloaded: true,
      is_legacy: true,
      is_retired: false,
    });
    expect(summarizeStt(local, [bin], bin.id, [], {}, t).ready).toBe(true);
  });

  test("other gaps keep their own issue", () => {
    const missing = model({ id: "nemotron" });
    expect(summarizeStt(local, [missing], missing.id, [], {}, t).issue).toBe(
      "not_downloaded",
    );
    expect(summarizeStt(local, [], "", [], {}, t).issue).toBe("no_model");
  });

  test("cloud transcription is not held back by a retired local selection", () => {
    const summary = summarizeStt(
      cloud,
      [retired],
      retired.id,
      [],
      { elevenlabs: true },
      t,
    );
    expect(summary.where).toBe("cloud");
    expect(summary.issue).not.toBe("needs_update");
  });
});

test("a finished download switches to a current model, never an older one", () => {
  expect(isAutoSelectable(model({ id: "nemotron" }))).toBe(true);
  expect(isAutoSelectable(retired)).toBe(false);
  expect(
    isAutoSelectable(
      model({ id: "medium", is_legacy: true, is_retired: false }),
    ),
  ).toBe(false);
});
