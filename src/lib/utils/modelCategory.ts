import type { ModelInfo } from "@/bindings";

/**
 * High-level grouping for the Models tab. Mirrors the backend `EngineType`:
 * transcription engines map to "stt", the bundled llama.cpp engine to "llm",
 * and the voices (Kokoro, and the native voice packs) to "tts".
 */
export type ModelCategory = "stt" | "llm" | "tts";

/** Derive the UI category for a model from its engine type. */
export const getModelCategory = (model: ModelInfo): ModelCategory => {
  switch (model.engine_type) {
    case "LlamaCpp":
      return "llm";
    case "Kokoro":
    case "NativeTts":
      return "tts";
    default:
      return "stt";
  }
};
