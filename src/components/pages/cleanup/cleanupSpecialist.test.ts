import { describe, expect, test } from "bun:test";
import type { AppSettings, ModelInfo, PostProcessReadiness } from "@/bindings";
import { findCleanupSpecialist } from "./cleanupSpecialist";

const model = (overrides: Partial<ModelInfo>): ModelInfo =>
  ({
    id: "model",
    name: "Model",
    is_cleanup_specialist: false,
    ...overrides,
  }) as ModelInfo;

const models = [
  model({
    id: "speakoflow-mini",
    name: "SpeakoFlow Mini",
    is_cleanup_specialist: true,
  }),
  model({ id: "gemma-3-1b", name: "Gemma 3 1B" }),
  // A user's own file whose name says nothing; the catalog flag is what counts.
  model({
    id: "local-abc123",
    name: "my-cleanup",
    is_cleanup_specialist: true,
  }),
];

const ready = (modelId: string): PostProcessReadiness => ({
  state: "ready",
  source: "dedicated_cleanup_selection",
  provider_id: "builtin",
  provider_label: "On this device",
  model: modelId,
});

const stored = (providerId: string, modelId: string) =>
  ({
    post_process_provider_id: providerId,
    post_process_models: { [providerId]: modelId },
  }) as unknown as AppSettings;

describe("findCleanupSpecialist", () => {
  test("a catalog fine-tune is found with its entry", () => {
    const found = findCleanupSpecialist(null, models, ready("speakoflow-mini"));
    expect(found?.model?.name).toBe("SpeakoFlow Mini");
  });

  test("the catalog flag counts even when the name does not say so", () => {
    expect(
      findCleanupSpecialist(null, models, ready("local-abc123"))?.model?.id,
    ).toBe("local-abc123");
  });

  test("a copy served by the user's own endpoint is found by its name", () => {
    const found = findCleanupSpecialist(
      null,
      models,
      ready("hf.co/someone/SpeakoFlow-Mini-GGUF:Q8_0"),
    );
    expect(found).toEqual({
      modelId: "hf.co/someone/SpeakoFlow-Mini-GGUF:Q8_0",
      model: null,
    });
  });

  test("a general-purpose model is not a specialist", () => {
    expect(findCleanupSpecialist(null, models, ready("gemma-3-1b"))).toBeNull();
  });

  test("the model that will run wins over the stored selection", () => {
    // Cleanup borrowed the assistant's general model; the stored fine-tune
    // selection is not what runs.
    expect(
      findCleanupSpecialist(
        stored("builtin", "speakoflow-mini"),
        models,
        ready("gemma-3-1b"),
      ),
    ).toBeNull();
  });

  test("with nothing ready, the stored selection decides", () => {
    const unavailable: PostProcessReadiness = {
      state: "unavailable",
      reason: "no_model_configured",
      source: null,
      provider_id: null,
      provider_label: null,
    };
    expect(
      findCleanupSpecialist(
        stored("builtin", "speakoflow-mini"),
        models,
        unavailable,
      )?.modelId,
    ).toBe("speakoflow-mini");
    expect(findCleanupSpecialist(null, models, null)).toBeNull();
  });
});
