import { describe, expect, test } from "bun:test";
import {
  baseLanguages,
  languageNames,
  pickGpu,
  prefersEnglish,
  recommend,
  SETUP_MODELS,
  SETUP_SPEECH_OPTIONS,
  voiceEngineFor,
  voiceModelId,
  type HardwareFacts,
} from "./recommend";

describe("setup's speech list", () => {
  test("is a short list where no two cards share a name or a model", () => {
    const names = SETUP_SPEECH_OPTIONS.map((option) => option.name);
    const ids = SETUP_SPEECH_OPTIONS.map((option) => option.id);
    expect(new Set(names).size).toBe(names.length);
    expect(new Set(ids).size).toBe(ids.length);
    expect(SETUP_SPEECH_OPTIONS.length).toBeLessThanOrEqual(6);
  });

  test("includes both models setup can recommend", () => {
    const ids = SETUP_SPEECH_OPTIONS.map((option) => option.id);
    expect(ids).toContain(SETUP_MODELS.speech.english);
    expect(ids).toContain(SETUP_MODELS.speech.multilingual);
  });
});

const base: HardwareFacts = {
  memoryGb: 16,
  gpu: null,
  cores: 8,
  unifiedMemory: false,
  webgpu: false,
  nativeVoice: true,
  prefersEnglish: true,
};

const facts = (overrides: Partial<HardwareFacts>): HardwareFacts => ({
  ...base,
  ...overrides,
});

describe("first-run recommendation", () => {
  test("a dedicated 8 GB card gets the larger assistant", () => {
    const rec = recommend(
      facts({
        memoryGb: 32,
        gpu: { name: "RTX 4070", kind: "dedicated", vramMb: 8188 },
        webgpu: true,
      }),
    );
    expect(rec.assistant).toEqual({
      choice: "balanced",
      enabled: false,
      reason: "gpu",
    });
    expect(rec.voice).toEqual({ route: "webview", enabled: false });
  });

  test("a 6 GB card is not enough for the larger model", () => {
    const rec = recommend(
      facts({ gpu: { name: "RTX 2060", kind: "dedicated", vramMb: 6144 } }),
    );
    expect(rec.assistant.choice).toBe("quick");
    expect(rec.assistant.enabled).toBe(false);
  });

  test("an integrated GPU never counts as strong, however much it shares", () => {
    const rec = recommend(
      facts({
        memoryGb: 64,
        gpu: { name: "Radeon 780M", kind: "integrated", vramMb: 16384 },
      }),
    );
    expect(rec.assistant.choice).toBe("quick");
  });

  test("Apple Silicon with 16 GB runs the larger model", () => {
    const rec = recommend(facts({ unifiedMemory: true, memoryGb: 16 }));
    expect(rec.assistant.choice).toBe("balanced");
    expect(rec.assistant.reason).toBe("unified");
  });

  test("8 GB of memory suggests the small model, switched off", () => {
    const rec = recommend(facts({ memoryGb: 8 }));
    expect(rec.assistant).toEqual({
      choice: "quick",
      enabled: false,
      reason: "tight",
    });
    // No assistant, no reason to download its voice.
    expect(rec.voice.enabled).toBe(false);
  });

  test("unknown memory still offers the small model", () => {
    const rec = recommend(facts({ memoryGb: 0 }));
    expect(rec.assistant).toEqual({
      choice: "quick",
      enabled: false,
      reason: "unknown",
    });
  });

  test("the voice moves off the web view without WebGPU", () => {
    expect(recommend(facts({ webgpu: false })).voice.route).toBe("native");
    expect(recommend(facts({ webgpu: false, cores: 4 })).voice.route).toBe(
      "light",
    );
    // Unknown core count is not a small processor.
    expect(recommend(facts({ cores: 0 })).voice.route).toBe("native");
  });

  test("with no GPU and no native engine the voice is offered but off", () => {
    const rec = recommend(facts({ webgpu: false, nativeVoice: false }));
    expect(rec.voice).toEqual({ route: "slow", enabled: false });
  });

  test("language decides speech model without opting into cleanup", () => {
    const english = recommend(facts({ prefersEnglish: true }));
    expect(english.speech).toBe("english");
    expect(english.cleanup).toBe(false);
    const other = recommend(facts({ prefersEnglish: false }));
    expect(other.speech).toBe("multilingual");
    expect(other.cleanup).toBe(false);
  });

  test("optional downloads stay off even on a capable machine", () => {
    for (const prefersEnglish of [true, false]) {
      const rec = recommend(
        facts({
          memoryGb: 64,
          gpu: { name: "RTX 4090", kind: "dedicated", vramMb: 24576 },
          webgpu: true,
          prefersEnglish,
        }),
      );
      expect(rec.cleanup).toBe(false);
      expect(rec.assistant.enabled).toBe(false);
      expect(rec.voice.enabled).toBe(false);
    }
  });
});

describe("helpers", () => {
  test("pickGpu prefers dedicated, then memory", () => {
    expect(pickGpu([])).toBeNull();
    expect(
      pickGpu([
        { name: "iGPU", kind: "integrated", total_vram_mb: 16000 },
        { name: "dGPU", kind: "dedicated", total_vram_mb: 8000 },
        { name: "dGPU big", kind: "dedicated", total_vram_mb: 16000 },
      ])?.name,
    ).toBe("dGPU big");
    expect(
      pickGpu([
        { name: "iGPU", kind: "integrated", total_vram_mb: 16000 },
        { name: "odd", kind: "unknown", total_vram_mb: 2000 },
      ])?.name,
    ).toBe("odd");
  });

  test("prefersEnglish reads only the primary locale", () => {
    expect(prefersEnglish(["en-GB"])).toBe(true);
    expect(prefersEnglish(["", "EN-us"])).toBe(true);
    // A second-listed English is how WebView2 reports most non-English systems.
    expect(prefersEnglish(["de-DE", "en-US"])).toBe(false);
    expect(prefersEnglish(["ne-NP", "hi"])).toBe(false);
    expect(prefersEnglish([])).toBe(false);
  });

  test("voice routes map to what the backend loads", () => {
    expect(voiceModelId("webview")).toBeNull();
    expect(voiceModelId("slow")).toBeNull();
    expect(voiceModelId("native")).toBe("kokoro-82m-native");
    expect(voiceModelId("light")).toBe("kitten-micro-0.8");
    expect(voiceEngineFor("light")).toEqual({
      engine: "kitten",
      model: "kitten-micro-0.8",
    });
    expect(voiceEngineFor("native")).toEqual({ engine: "kokoro", model: null });
  });
});

describe("languageNames", () => {
  test("names each language in the UI language, common ones first", () => {
    const names = languageNames(["nl", "ja", "en", "es"], "en");
    expect(names).toEqual(["English", "Spanish", "Japanese", "Dutch"]);
  });

  test("follows the UI language", () => {
    expect(languageNames(["de"], "de")).toEqual(["Deutsch"]);
  });

  test("drops duplicates and keeps an unknown code rather than losing it", () => {
    const names = languageNames(["en", "en", "qq"], "en");
    expect(names[0]).toBe("English");
    expect(names).toHaveLength(2);
  });
});

describe("baseLanguages", () => {
  test("regional variants count once", () => {
    expect(baseLanguages(["en-US", "en-GB", "es-ES", "es-419", "fr"])).toEqual([
      "en",
      "es",
      "fr",
    ]);
  });

  test("names come out once per language, not per region", () => {
    expect(languageNames(["en-US", "en-GB", "pt-BR", "pt-PT"], "en")).toEqual([
      "English",
      "Portuguese",
    ]);
  });
});
