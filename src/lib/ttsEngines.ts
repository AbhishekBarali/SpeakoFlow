import type { AppSettings } from "@/bindings";

/**
 * The voice engines, described once for every surface that shows them: the
 * Voice settings, the model picker, the setup dialog and the Models summary.
 * Mirrors `TTS_PROVIDERS` in `src-tauri/src/tts.rs`; the order is the order
 * they are shown in.
 *
 * Examples are placeholders only and are never saved on their own. Where the
 * user leaves a field empty the backend falls back to the engine's default,
 * which is what the example shows.
 */
export interface TtsEngineSpec {
  id: string;
  /** Synthesizes on this computer (Kokoro, and the native voices). */
  local?: boolean;
  /** An address the user supplies. Hosted engines have a fixed one. */
  url?: { required: boolean; example: string };
  /** Whether the engine needs a key. `optional` for a self-hosted server. */
  key: "required" | "optional" | "none";
  /** Where to get a key. */
  keyUrl?: string;
  /** A model field. Omitted where the voice implies the model. */
  model?: { example: string };
  /** The voice field. `required` where no voice can be assumed. */
  voice: { required: boolean; example: string };
  /** Speed range the engine accepts, or null when it has no speed control. */
  speed: [number, number] | null;
}

export const TTS_ENGINES: readonly TtsEngineSpec[] = [
  {
    id: "kokoro",
    local: true,
    key: "none",
    voice: { required: false, example: "af_heart" },
    speed: [0.25, 4],
  },
  {
    // The small voice: runs on the processor through the native engine
    // (native_tts.rs), in three sizes (nano, micro, mini).
    id: "kitten",
    local: true,
    key: "none",
    voice: { required: false, example: "Bella" },
    speed: [0.25, 4],
  },
  {
    // Kyutai's Pocket TTS, native. Each voice is cloned from a short
    // recording, and the model follows that recording's pace, so it has no
    // speed control.
    id: "pocket",
    local: true,
    key: "none",
    voice: { required: false, example: "Mary" },
    speed: null,
  },
  {
    // Supertone's Supertonic 3, native.
    id: "supertonic",
    local: true,
    key: "none",
    voice: { required: false, example: "F1" },
    speed: [0.5, 2],
  },
  {
    id: "openai",
    key: "required",
    keyUrl: "https://platform.openai.com/api-keys",
    model: { example: "gpt-4o-mini-tts" },
    voice: { required: false, example: "alloy" },
    speed: [0.25, 4],
  },
  {
    id: "elevenlabs",
    key: "required",
    keyUrl: "https://elevenlabs.io/app/settings/api-keys",
    model: { example: "eleven_flash_v2_5" },
    // An ElevenLabs voice belongs to an account; none can be assumed.
    voice: { required: true, example: "JBFqnCBsd6RMkjVDRZzb" },
    speed: [0.7, 1.2],
  },
  {
    id: "openrouter",
    key: "required",
    keyUrl: "https://openrouter.ai/keys",
    model: { example: "google/gemini-3.1-flash-tts-preview" },
    voice: { required: false, example: "Kore" },
    speed: [0.25, 4],
  },
  {
    id: "deepgram",
    key: "required",
    keyUrl: "https://console.deepgram.com/",
    // Deepgram's voice is its model id, so there is no separate model field.
    voice: { required: false, example: "aura-2-thalia-en" },
    speed: [0.7, 1.5],
  },
  {
    id: "cartesia",
    key: "required",
    keyUrl: "https://play.cartesia.ai/keys",
    model: { example: "sonic-3.6" },
    voice: { required: false, example: "db6b0ed5-d5d3-463d-ae85-518a07d3c2b4" },
    speed: [0.6, 1.5],
  },
  {
    id: "google",
    key: "required",
    keyUrl: "https://console.cloud.google.com/apis/credentials",
    voice: { required: false, example: "en-US-Chirp3-HD-Kore" },
    speed: [0.25, 2],
  },
  {
    id: "azure",
    url: {
      required: true,
      example: "https://eastus2.tts.speech.microsoft.com",
    },
    key: "required",
    keyUrl: "https://portal.azure.com/",
    voice: { required: false, example: "en-US-JennyNeural" },
    speed: [0.5, 2],
  },
  {
    id: "groq",
    key: "required",
    keyUrl: "https://console.groq.com/keys",
    model: { example: "canopylabs/orpheus-v1-english" },
    voice: { required: false, example: "troy" },
    speed: null,
  },
  {
    id: "xai",
    key: "required",
    keyUrl: "https://console.x.ai",
    voice: { required: false, example: "eve" },
    speed: [0.7, 1.5],
  },
  {
    id: "mistral",
    key: "required",
    keyUrl: "https://console.mistral.ai/api-keys",
    model: { example: "voxtral-mini-tts-2603" },
    // `voice_id` is required and the preset ids are opaque.
    voice: { required: true, example: "" },
    speed: null,
  },
  {
    id: "inworld",
    key: "required",
    keyUrl: "https://platform.inworld.ai",
    model: { example: "inworld-tts-2" },
    voice: { required: false, example: "Dennis" },
    speed: [0.5, 1.5],
  },
  {
    id: "custom",
    url: { required: true, example: "http://localhost:8880/v1" },
    key: "optional",
    model: { example: "tts-1" },
    voice: { required: false, example: "alloy" },
    speed: [0.25, 4],
  },
];

export const TTS_ENGINE_IDS: readonly string[] = TTS_ENGINES.map((e) => e.id);

/** The spec for an engine id, or undefined for one this build doesn't know. */
export const ttsEngineSpec = (id: string): TtsEngineSpec | undefined =>
  TTS_ENGINES.find((engine) => engine.id === id);

/**
 * Whether an ElevenLabs model performs audio tags (`[laughs]`, `[applause]`).
 * Mirrors `audio_tags::model_supports_tags` in `src-tauri/src/audio_tags.rs`:
 * the id's `v<N>` segment is generation 3 or later (`eleven_v3`,
 * `eleven_v3_conversational`, `eleven_v4`, `eleven_v4_turbo`). An empty
 * model falls back to the engine default, which does not.
 */
export const elevenLabsModelSupportsAudioTags = (model: string): boolean =>
  model
    .trim()
    .toLowerCase()
    .split(/[_-]/)
    .some((segment) => {
      const match = /^v(\d+)$/.exec(segment);
      return match !== null && Number(match[1]) >= 3;
    });

export interface TtsValues {
  url: string;
  key: string;
  model: string;
  voice: string;
}

/**
 * An engine's saved endpoint, key, model and voice. The engine in use keeps
 * its values in the flat fields (which older stores may hold alone); every
 * other engine keeps them in the per-engine maps.
 */
export const ttsValues = (
  settings: AppSettings | null | undefined,
  engine: string,
): TtsValues => {
  const active = settings?.assistant_tts_engine === engine;
  const pick = (
    flat: string | undefined,
    map: Partial<Record<string, string>> | undefined,
  ) => ((active ? flat : map?.[engine]) ?? "").trim();
  return {
    url: pick(
      settings?.assistant_tts_base_url,
      settings?.assistant_tts_base_urls,
    ),
    key: pick(
      settings?.assistant_tts_api_key,
      settings?.assistant_tts_api_keys,
    ),
    model: pick(settings?.assistant_tts_model, settings?.assistant_tts_models),
    voice: pick(
      settings?.assistant_tts_remote_voice,
      settings?.assistant_tts_remote_voices,
    ),
  };
};

/** Whether an engine still needs something before it can speak. */
export const ttsNeedsSetup = (
  settings: AppSettings | null | undefined,
  engine: string,
): boolean => {
  const spec = ttsEngineSpec(engine);
  if (!spec || spec.local) return false;
  const values = ttsValues(settings, engine);
  if (spec.url?.required && !values.url) return true;
  if (spec.key === "required" && !values.key) return true;
  return spec.voice.required && !values.voice;
};

/** Host of a URL, for the "Get a key at …" link. */
export const hostOf = (url: string): string => {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return url;
  }
};
